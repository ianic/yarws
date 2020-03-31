// server.rs
use base64;
use sha1::{Digest, Sha1};
use std::io;
use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use std::str;

#[derive(Debug)]
struct HTTPHeader {
    connection: String,
    upgrade: String,
    version: String,
    key: String,
    extensions: String,
}

impl HTTPHeader {
    fn new() -> HTTPHeader {
        HTTPHeader {
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
                _ => (),
                //_ => println!("other header: '{}' => '{}'", key, value),
            }
        }
    }

    fn is_ws_upgrade(&self) -> bool {
        self.connection == "Upgrade"
            && self.upgrade == "websocket"
            && self.version == "13"
            && self.key.len() > 0
    }

    fn upgrade_response(&self) -> String {
        let mut s = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_string();
        s.push_str(&ws_accept(&self.key));
        s.push_str(&"\r\n\r\n");
        s
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

fn parse_http_header(line: &str) -> Option<(&str, &str)> {
    let mut splitter = line.splitn(2, ':');
    let first = splitter.next()?;
    let second = splitter.next()?;
    Some((first, second.trim()))
}

fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    //println!("handle_connection");
    let mut rdr = io::BufReader::new(&stream);
    let mut header = HTTPHeader::new();
    loop {
        let mut line = String::new();
        // TODO: sta ako ovdje nikda nista ne posalje blokira mi thread !!!
        match rdr.read_line(&mut line) {
            Ok(0) => break, // eof
            Ok(2) => break, // empty line \r\n = end of header line
            Ok(_n) => header.parse(&line),
            _ => break,
        }
    }

    if header.is_ws_upgrade() {
        stream.write_all(header.upgrade_response().as_bytes())?;
        stream.flush()?;
        return handle_ws_connection(stream);
    }
    //println!("bad request");
    stream.write_all(BAD_REQUEST_HTTP_RESPONSE)?;
    stream.flush()?;
    Ok(())
}
const BAD_REQUEST_HTTP_RESPONSE: &[u8] = "HTTP/1.1 400 Bad Request\r\n\r\n".as_bytes();

fn handle_ws_connection(stream: TcpStream) -> io::Result<()> {
    println!("ws open");
    let mut output = stream.try_clone()?;
    let mut input = stream;

    let n = output.write(&a_frame())?;
    println!("ws written {} bytes", n);
    output.flush()?;

    loop {
        let mut buf = [0u8; 2];
        input.read_exact(&mut buf)?;

        let mut h = WsHeader::new(buf[0], buf[1]);
        let rn = h.read_next() as usize;
        if rn > 0 {
            let mut buf = vec![0u8; rn];
            input.read_exact(&mut buf)?;
            h.set_header(&buf);
        }

        let rn = h.payload_len as usize;
        if rn > 0 {
            let mut buf = vec![0u8; rn];
            input.read_exact(&mut buf)?;
            h.set_payload(&buf);
        }

        match h.kind() {
            WsFrameKind::Text => println!("ws body {} bytes, as str: {}", rn, h.payload_str()),
            WsFrameKind::Binary => println!("ws body is binary frame of size {}", h.payload_len),
            WsFrameKind::Close => {
                println!("ws close");
                output.write(close_frame().as_slice())?; // zasto ovdje zovem as_slice
                break;
            }
            WsFrameKind::Ping => {
                println!("ws ping");
                output.write(&h.to_pong())?;
            }
            WsFrameKind::Pong => println!("ws pong"),
            WsFrameKind::Continuation => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ws continuation frame not supported",
                ));
            }
            WsFrameKind::Reserved(opcode) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("reserved ws frame opcode {}", opcode),
                ));
            }
        }
    }
    Ok(())
}

struct WsHeader {
    fin: bool,
    rsv1: bool,
    rsv2: bool,
    rsv3: bool,
    mask: bool,
    opcode: u8,
    payload_len: u64,
    masking_key: [u8; 4],
    payload: Vec<u8>,
}

enum WsFrameKind {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
    Reserved(u8),
}

impl WsHeader {
    fn new(byte1: u8, byte2: u8) -> WsHeader {
        WsHeader {
            fin: byte1 & 0b1000_0000u8 != 0,
            rsv1: byte1 & 0b0100_0000u8 != 0,
            rsv2: byte1 & 0b0010_0000u8 != 0,
            rsv3: byte1 & 0b0001_0000u8 != 0,
            opcode: byte1 & 0b0000_1111u8,
            mask: byte2 & 0b1000_0000u8 != 0,
            payload_len: (byte2 & 0b0111_1111u8) as u64,
            masking_key: [0; 4],
            payload: vec![0; 0],
        }
    }
    fn read_next(&self) -> u8 {
        let mut n: u8 = if self.mask { 4 } else { 0 };
        if self.payload_len >= 126 {
            n += 2;
        }
        if self.payload_len == 127 {
            n += 4;
        }
        n
    }
    fn set_header(&mut self, buf: &[u8]) {
        let mask_start = buf.len() - 4;
        if mask_start == 8 {
            let bytes: [u8; 8] = [
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ];
            self.payload_len = u64::from_be_bytes(bytes);
        }
        if mask_start == 2 {
            let bytes: [u8; 2] = [buf[0], buf[1]];
            self.payload_len = u16::from_be_bytes(bytes) as u64;
        }
        for i in 0..4 {
            self.masking_key[i] = buf[mask_start + i];
        }
    }
    fn set_payload(&mut self, buf: &[u8]) {
        let mut decoded = vec![0u8; self.payload_len as usize];
        for (i, b) in buf.iter().enumerate() {
            decoded[i] = b ^ self.masking_key[i % 4];
        }
        self.payload = decoded;
    }
    fn payload_str(&self) -> &str {
        match str::from_utf8(&self.payload) {
            Ok(v) => v,
            _ => "",
        }
    }
    fn kind(&self) -> WsFrameKind {
        match self.opcode {
            0 => WsFrameKind::Continuation,
            1 => WsFrameKind::Text,
            2 => WsFrameKind::Binary,
            8 => WsFrameKind::Close,
            9 => WsFrameKind::Ping,
            0xa => WsFrameKind::Pong,
            _ => WsFrameKind::Reserved(self.opcode),
        }
    }
    fn to_pong(&self) -> [u8; 2] {
        let buf = [0b1000_1010u8, 0b00000000u8];
        buf
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

fn close_frame() -> Vec<u8> {
    vec![0b1000_1000u8, 0b0000_0000u8]
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

var socket = new WebSocket('ws://localhost:8000');
socket.addEventListener('open', function (event) {
    console.log('open');
    //socket.send('first');
    //socket.send('second');
    //socket.send('third');
    socket.close();
});
socket.addEventListener('message', function (event) {
    console.log('Message from server ', event.data);
});
socket.addEventListener('close', function (event) {
    console.log('ws closed');
});
socket.addEventListener('error', function (event) {
  console.log('ws error: ', event);
});


*/
