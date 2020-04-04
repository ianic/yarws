use crate::http;
use inflate::inflate_bytes;
use std::error::Error;
use std::str;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};

pub async fn handle(hr: http::Response) {
    println!("ws open");
    let deflate_supported = hr.deflate_supported;
    let (input, output) = io::split(hr.stream);
    let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel(16);

    spawn(async move {
        if let Err(e) = write(output, rx).await {
            println!("ws_write error {:?}", e);
        }
        println!("write half closed");
    });
    spawn(async move {
        if let Err(e) = read(input, tx, deflate_supported).await {
            println!("ws_read error {:?}", e);
        }
        println!("read half closed");
    });
}

async fn read(
    mut input: ReadHalf<TcpStream>,
    mut rx: Sender<Vec<u8>>,
    deflate_supported: bool,
) -> Result<(), Box<dyn Error>> {
    let mut continuation = Frame::empty();
    let mut header_buf = [0u8; 12];
    loop {
        // read first two bytes
        if let Err(e) = input.read_exact(&mut header_buf[0..2]).await {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                break; //tcp closed without ws close handshake
            }
            return Err(Box::new(e));
        }
        let mut frame = Frame::new(header_buf[0], header_buf[1]);
        // read rest of the header
        let hl = frame.header_len() as usize;
        if hl > 0 {
            input.read_exact(&mut header_buf[2..hl]).await?;
            frame.set_header(&header_buf[2..hl]);
        }
        // read payload
        let pl = frame.payload_len as usize;
        if pl > 0 {
            let mut buf = vec![0u8; pl];
            input.read_exact(&mut buf).await?;
            frame.set_payload(&buf);
        }

        if !frame.is_valid(deflate_supported, !continuation.is_empty()) {
            println!("invalid frame {:?}", frame);
            break;
        }
        if !frame.is_control_frame() {
            if frame.is_start() {
                continuation = frame;
                continue;
            } else if frame.is_continuation() {
                continuation.append(&frame);
                if !frame.is_end() {
                    continue;
                }
                frame = continuation;
                continuation = Frame::empty();
            }
        }
        if !frame.validate_payload() {
            println!("invalid payload");
            break;
        }
        // handle message
        match frame.opcode {
            TEXT => {
                //println!("ws body {} bytes, as str: {}", frame.payload_len, frame.payload_str());
                println!("ws body {} bytes", frame.payload_len);
                rx.send(text_message(frame.payload_str())).await?;
            }
            BINARY => {
                println!(
                    "ws body is binary frame of len before: {}, after: {}, capacity: {}",
                    frame.payload_len,
                    frame.payload.len(),
                    frame.payload.capacity()
                );
                rx.send(binary_message(&frame.payload)).await?;
            }
            CLOSE => {
                println!("ws close");
                rx.send(close_message()).await?;
                break;
            }
            PING => {
                println!("ws ping");
                rx.send(frame.to_pong()).await?;
            }
            PONG => println!("ws pong"),
            _ => {
                return Err(format!("reserved ws frame opcode {}", frame.opcode).into());
            }
        }
    }
    Ok(())
}

async fn write(mut output: WriteHalf<TcpStream>, mut rx: Receiver<Vec<u8>>) -> io::Result<()> {
    while let Some(v) = rx.recv().await {
        output.write(&v).await?;
    }
    Ok(())
}

#[derive(Debug)]
struct Frame {
    fin: bool,
    rsv1: bool,
    rsv: u8,
    mask: bool,
    opcode: u8,
    payload_len: u64,
    masking_key: [u8; 4],
    payload: Vec<u8>,
}

// data frame types
const CONTINUATION: u8 = 0;
const TEXT: u8 = 1;
const BINARY: u8 = 2;
// constrol frame types
const CLOSE: u8 = 8;
const PING: u8 = 9;
const PONG: u8 = 10;

impl Frame {
    fn new(byte1: u8, byte2: u8) -> Frame {
        Frame {
            fin: byte1 & 0b1000_0000u8 != 0,
            rsv1: byte1 & 0b0100_0000u8 != 0,
            rsv: (byte1 & 0b0111_0000u8) >> 4,
            opcode: byte1 & 0b0000_1111u8,
            mask: byte2 & 0b1000_0000u8 != 0,
            payload_len: (byte2 & 0b0111_1111u8) as u64,
            masking_key: [0; 4],
            payload: vec![0; 0],
        }
    }
    fn empty() -> Frame {
        Frame::new(0, 0)
    }
    fn header_len(&self) -> u8 {
        let mut n: u8 = if self.mask { 4 } else { 0 };
        if self.payload_len >= 126 {
            n += 2;
        }
        if self.payload_len == 127 {
            n += 6;
        }
        n
    }
    fn set_header(&mut self, buf: &[u8]) {
        let mask_start = buf.len() - 4;
        if mask_start == 8 {
            let bytes: [u8; 8] = [buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]];
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
    fn is_data_frame(&self) -> bool {
        self.opcode == TEXT || self.opcode == BINARY
    }
    fn is_control_frame(&self) -> bool {
        self.opcode >= CLOSE && self.opcode <= PONG
    }
    fn is_rsv_ok(&self, deflate_supported: bool) -> bool {
        if deflate_supported {
            return self.rsv == 0 || self.rsv == 4;
        }
        // rsv must be 0, when no extension defining RSV meaning has been negotiated
        self.rsv == 0
    }
    fn is_valid(&self, deflate_supported: bool, in_continuation: bool) -> bool {
        if self.is_control_frame() && self.payload_len > 125 {
            return false;
        }
        if self.is_control_frame() && !self.fin {
            return false; // Control frames themselves MUST NOT be fragmented.
        }
        if !self.is_control_frame() {
            if !in_continuation && self.is_part() {
                return false;
            }
            if in_continuation && !self.is_end() && !self.is_part() {
                return false;
            }
        }
        self.is_rsv_ok(deflate_supported)
    }
    fn validate_payload(&mut self) -> bool {
        if !self.inflate() {
            println!("undeflate failed");
            return false;
        }
        if self.opcode != TEXT {
            return true;
        }
        str::from_utf8(&self.payload).is_ok()
    }
    fn is_start(&self) -> bool {
        !self.fin && self.is_data_frame()
    }
    fn is_end(&self) -> bool {
        self.fin && self.opcode == CONTINUATION
    }
    fn is_part(&self) -> bool {
        !self.fin && self.opcode == CONTINUATION
    }
    fn is_continuation(&self) -> bool {
        self.opcode == CONTINUATION
    }
    fn is_empty(&self) -> bool {
        !self.fin && self.opcode == 0
    }
    fn to_pong(&self) -> Vec<u8> {
        create_message(PONG, &self.payload)
    }
    fn append(&mut self, other: &Frame) {
        self.payload_len = self.payload_len + other.payload_len;
        self.payload.extend_from_slice(&other.payload);
    }
    fn inflate(&mut self) -> bool {
        if !self.rsv1 || self.payload_len == 0 {
            return true;
        }
        match inflate_bytes(&self.payload) {
            Ok(p) => {
                self.payload = p;
                true
            }
            Err(e) => {
                println!("decompress error {:?}", e);
                false
            }
        }
    }
}

fn close_message() -> Vec<u8> {
    vec![0b1000_1000u8, 0b0000_0000u8]
}

fn text_message(text: &str) -> Vec<u8> {
    create_message(TEXT, text.as_bytes())
}

fn binary_message(data: &[u8]) -> Vec<u8> {
    create_message(BINARY, data)
}

/*
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-------+-+-------------+-------------------------------+
|F|R|R|R| opcode|M| Payload len |    Extended payload length    |
|I|S|S|S|  (4)  |A|     (7)     |             (16/64)           |
|N|V|V|V|       |S|             |   (if payload len==126/127)   |
| |1|2|3|       |K|             |                               |
+-+-+-+-+-------+-+-------------+ - - - - - - - - - - - - - - - +
|     Extended payload length continued, if payload len == 127  |
+ - - - - - - - - - - - - - - - +-------------------------------+
|                               |Masking-key, if MASK set to 1  |
+-------------------------------+-------------------------------+
| Masking-key (continued)       |          Payload Data         |
+-------------------------------- - - - - - - - - - - - - - - - +
:                     Payload Data continued ...                :
+ - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - +
|                     Payload Data continued ...                |
+---------------------------------------------------------------+
*/
fn create_message(opcode: u8, body: &[u8]) -> Vec<u8> {
    let mut buf = vec![0b1000_0000u8 + opcode];

    // add peyload length
    let l = body.len();
    if l < 126 {
        buf.push(l as u8);
    } else if body.len() < 65536 {
        buf.push(126u8);
        let l = l as u16;
        buf.extend_from_slice(&l.to_be_bytes());
    } else {
        buf.push(127u8);
        let l = l as u64;
        buf.extend_from_slice(&l.to_be_bytes());
    }

    buf.extend_from_slice(body);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::compress_to_vec;
    use miniz_oxide::inflate::decompress_to_vec;
    #[test]
    fn test_text_message() {
        let buf = text_message("abc");
        assert_eq!(5, buf.len());
        assert_eq!([0x81, 0x03, 0x61, 0x62, 0x63], buf[0..]);

        let buf = text_message("The length of the Payload data, in bytes: if 0-125, that is the payload length.  If 126, the following 2 bytes interpreted as a 16-bit unsigned integer are the payload length.");
        assert_eq!(179, buf.len());
        assert_eq!([0x81, 0x7e, 0x00, 0xaf], buf[0..4]);
        assert_eq!(0xaf, buf[3]); // 175 is body length

        //println!("{:02x?}", buf);
    }
    #[test]
    fn test_compress_decompress() {
        let data = "hello world";
        let compressed = compress_to_vec(data.as_bytes(), 6);
        let decompressed = decompress_to_vec(compressed.as_slice()).expect("Failed to decompress!");
        assert_eq!(data, str::from_utf8(&decompressed).unwrap());
        //println!("{:02x?}", data);
        //println!("{:02x?}", decompressed);
    }
    #[test]
    fn frame_inflate() {
        let mut f = Frame::new(0, 0);
        f.rsv1 = true;
        f.rsv = 4;
        f.payload_len = 7;
        f.payload = vec![0xf2, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00];
        //f.payload = vec![0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x00, 0x00, 0xff, 0xff];
        assert_eq!(true, f.inflate());
        assert_eq!("Hello", f.payload_str());
    }
}
