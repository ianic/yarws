// server.rs
use base64;
use sha1::{Digest, Sha1};
use std::convert::TryInto;
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

fn parse_http_header(line: &str) -> Option<(&str, &str)> {
    let mut splitter = line.splitn(2, ':');
    let first = splitter.next()?;
    let second = splitter.next()?;
    Some((first, second.trim()))
}

fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    //println!("handle_connection");
    //let mut ostream = stream.try_clone()?;
    let mut rdr = io::BufReader::new(&stream);

    let mut header = Header::new();
    loop {
        let mut line = String::new();

        // TODO: sta ako ovdje nikda nista ne posalje blokira mi thread !!!
        match rdr.read_line(&mut line) {
            Ok(0) => break, // eof
            Ok(2) => break, // empty line \r\n = end of header line
            Ok(_n) => {
                //print!("line: {}", line);
                header.parse(&line);
            }
            _ => break,
        }
    }

    if header.is_ws_upgrade() {
        let rsp = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ".to_string() + &ws_accept(&header.key) + "\r\n\r\n";
        stream.write_all(rsp.as_bytes())?;
        stream.flush()?;
        //ostream.write(a_frame().as_slice())?;
        return handle_ws_connection(stream);
    } else {
        //println!("bad request");
        //println!("header: {:?}", header);
        //println!("is websocket upgrade: {}", header.is_ws_upgrade());
        //let rsp = "HTTP/1.1 200 OK\r\n\r\n{}";
        let rsp = "HTTP/1.1 400 Bad Request\r\n\r\n";
        stream.write_all(rsp.as_bytes())?;
        stream.flush()?;
    }
    Ok(())
}

fn handle_ws_connection(stream: TcpStream) -> io::Result<()> {
    println!("ws open");
    let mut output = stream.try_clone()?;
    let mut input = stream;

    let n = output.write(a_frame().as_slice())?;
    println!("ws written {} bytes", n);
    output.flush()?;

    loop {
        let mut buf = [0u8; 2];
        let n = input.read(&mut buf)?;
        if n != 2 {
            // TODO: error missing header
            break;
        }
        let mut h = WsHeader::new(buf[0], buf[1]);

        let rn = h.read_next() as usize;
        if rn > 0 {
            let mut buf = vec![0u8; rn];
            let n = input.read(&mut buf)?;
            if n != rn {
                // TODO: error missing header
                break;
            }
            h.set_header(&buf);
        }

        let rn = h.payload_len as usize;
        if h.payload_len > 0 {
            let mut buf = vec![0u8; rn];
            let n = input.read(&mut buf)?;
            if n != rn {
                // TODO: error missing header
                break;
            }
            h.set_payload(&buf);
            println!("ws body {} bytes, as str: {}", n, h.payload_str());
        }

        if h.is_close() {
            println!("ws close request recieved");
            output.write(close_frame().as_slice())?; // zasto ovdje zovem as_slice
            break;
        }
    }

    Ok(())
}

fn is_close(buf: &[u8]) -> bool {
    let msk = 0b0000_1111u8;
    buf[0] & msk == 0x8
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
    fn is_close(&self) -> bool {
        self.opcode == 8
    }
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
