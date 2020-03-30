// server.rs
use base64;
use sha1::{Digest, Sha1};
use std::io;
use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use std::str;

#[derive(Debug)]
struct Header {
    connection: String,
    upgrade: String,
    version: String,
    key: String,
    extensions: String,
}

impl Header {
    fn new() -> Header {
        Header {
            connection: String::new(),
            upgrade: String::new(),
            version: String::new(),
            key: String::new(),
            extensions: String::new(),
        }
    }

    fn parse(&mut self, line: &str) {
        if let Some((key, value)) = parse_http_header(&line) {
            match key {
                "Connection" => self.connection = value.to_string(),
                "Upgrade" => self.upgrade = value.to_string(),
                "Sec-WebSocket-Version" => self.version = value.to_string(),
                "Sec-WebSocket-Key" => self.key = value.to_string(),
                "Sec-WebSocket-Extensions" => self.extensions = value.to_string(),
                //"Host"
                //"Origin"
                _ => (), // println!("other: '{}' => '{}'", key, value),
            }
        }
    }

    fn is_ws_upgrade(&self) -> bool {
        self.connection == "Upgrade"
            && self.upgrade == "websocket"
            && self.version == "13"
            && self.key.len() > 0
    }
}
fn a_frame() -> Vec<u8> {
    let mut buf = vec![0b10000001u8, 0b00000100u8];
    for b in "pero".as_bytes() {
        buf.push(*b);
    }
    //buf.append("pero".as_bytes());
    buf
}
fn parse_http_header(line: &str) -> Option<(&str, &str)> {
    let mut splitter = line.splitn(2, ':');
    let first = splitter.next()?;
    let second = splitter.next()?;
    Some((first, second.trim()))
}

fn handle_connection(stream: TcpStream) -> io::Result<()> {
    let mut ostream = stream.try_clone()?;
    let mut rdr = io::BufReader::new(stream);

    let mut header = Header::new();
    loop {
        let mut line = String::new();
        match rdr.read_line(&mut line) {
            Ok(0) => break, // eof
            Ok(2) => break, // empty line \r\n = end of header line
            Ok(_n) => {
                header.parse(&line);
            }
            _ => break,
        }
    }

    if header.is_ws_upgrade() {
        let rsp = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_string() + &ws_accept(&header.key) + "\r\n\r\n";
        ostream.write_all(rsp.as_bytes())?;
        ostream.write(a_frame().as_slice())?;
    } else {
        println!("header: {:?}", header);
        println!("is websocket upgrade: {}", header.is_ws_upgrade());
        let rsp = "HTTP/1.1 200 OK\r\n\r\n{}";
        ostream.write_all(rsp.as_bytes())?;
    }
    ostream.flush()?;
    Ok(())
}

fn main() {
    let listener = TcpListener::bind("127.0.0.1:8000").expect("could not start server");

    // accept connections and get a TcpStream
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream) {
                    println!("error {:?}", e);
                }
            }
            Err(e) => {
                print!("connection failed {}\n", e);
            }
        }
    }
}

const WS_MAGIC_KEY: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

fn ws_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    let s = key.to_string() + WS_MAGIC_KEY;

    hasher.input(s.as_bytes());
    let hr = hasher.result();
    base64::encode(&hr)
}

#[cfg(test)]
#[macro_use]
extern crate hex_literal;

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
    fn test_frame() {
        let mut buf = vec![0b10000001u8, 0b00000001u8];
        buf.push("a".as_bytes()[0]);
        println!("buf: {:?}", buf)
    }
}

/*
example of ws upgrade header:

16 GET / HTTP/1.1
22 Host: localhost:8000
21 Connection: Upgrade
18 Pragma: no-cache
25 Cache-Control: no-cache
135 User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/80.0.3987.149 Safari/537.36
20 Upgrade: websocket
31 Origin: http://localhost:8000
27 Sec-WebSocket-Version: 13
36 Accept-Encoding: gzip, deflate, br
114 Accept-Language: en-US,en;q=0.9,de;q=0.8,et;q=0.7,hr;q=0.6,it;q=0.5,sk;q=0.4,sl;q=0.3,sr;q=0.2,bs;q=0.1,mt;q=0.1
45 Sec-WebSocket-Key: HNDy4+PhhRtPmNt1Xet/Ew==
70 Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits
*/
/*

const socket = new WebSocket('ws://localhost:8000');
socket.addEventListener('open', function (event) {
    socket.send('Hello Server!');
});
socket.addEventListener('message', function (event) {
    console.log('Message from server ', event.data);
});
socket.addEventListener('close', function (event) {
    console.log('ws closed');
});

*/
