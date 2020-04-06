use crate::http;
use inflate::inflate_bytes;
use std::error::Error;
use std::str;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf};
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
struct Msg {
    is_binary: bool,
    binary: Vec<u8>,
    text: String,
}
impl Msg {
    fn as_raw(&self) -> Vec<u8> {
        if self.is_binary {
            return binary_message(&self.binary);
        }
        text_message(&self.text)
    }
}

pub async fn handle(hu: http::Upgrade, log: slog::Logger) {
    debug!(log, "open");
    let (soc_rx, mut soc_tx) = io::split(hu.stream);
    let (raw_tx, mut raw_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel(16);
    let (session_tx, session_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(16);
    let (conn_tx, mut conn_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(16);
    let mut raw_tx2 = raw_tx.clone();

    // writes raw ws frames to the tcp socket
    // raw_rx -> soc_tx
    let l = log.clone();
    spawn(async move {
        while let Some(v) = raw_rx.recv().await {
            if let Err(e) = soc_tx.write(&v).await {
                error!(l, "{}", e);
                break;
            }
        }
        debug!(l, "write half closed");
    });

    // reads tcp socket, parse incoming stream as ws frames and then into application messages
    // soc_rx -> raw_tx       - for control frames
    //        -> session_tx   - for data frames
    let l = log.clone();
    let deflate_supported = hu.deflate_supported;
    spawn(async move {
        if let Err(e) = read(soc_rx, raw_tx, session_tx, deflate_supported, &l).await {
            error!(l, "{}", e);
        }
        debug!(l, "read half closed");
    });

    // transforms application Msg to raw
    // conn_rx -> raw_tx
    let l = log.clone();
    spawn(async move {
        while let Some(m) = conn_rx.recv().await {
            if let Err(e) = raw_tx2.send(m.as_raw()).await {
                error!(l, "{}", e);
                break;
            }
        }
        debug!(l, "msg to raw closed");
    });

    // process application messages
    // session_rx -> conn_tx
    let l = log.clone();
    spawn(async move {
        if let Err(e) = session(session_rx, conn_tx).await {
            error!(l, "{}", e);
        }
        debug!(l, "session closed");
    });
}

// TODO: kako iz session signalizirati close
async fn session(mut rx: Receiver<Msg>, mut tx: Sender<Msg>) -> Result<(), Box<dyn Error>> {
    while let Some(m) = rx.recv().await {
        // some serious processing
        //m.text = m.text.chars().rev().collect::<String>();
        // if m.text == "close" {
        //     break;
        // }
        tx.send(m).await?;
    }
    Ok(())
}

async fn read_payload(soc_rx: &mut ReadHalf<TcpStream>, frame: &mut Frame) -> io::Result<()> {
    let l = frame.payload_len as usize;
    if l > 0 {
        let mut buf = vec![0u8; l];
        soc_rx.read_exact(&mut buf).await?;
        frame.set_payload(buf.to_vec());
    }
    Ok(())
}

async fn read_header(soc_rx: &mut ReadHalf<TcpStream>, buf: &mut [u8]) -> io::Result<Option<Frame>> {
    if let Err(e) = soc_rx.read_exact(&mut buf[0..2]).await {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(e);
    }
    let mut frame = Frame::new(buf[0], buf[1]);

    if let Some(l) = frame.header_len() {
        let b = &mut buf[2..l + 2];
        soc_rx.read_exact(b).await?;
        frame.set_header(b);
    }
    Ok(Some(frame))
}

async fn read(
    mut soc_rx: ReadHalf<TcpStream>,
    mut raw_tx: Sender<Vec<u8>>,
    mut session_tx: Sender<Msg>,
    deflate_supported: bool,
    log: &slog::Logger,
) -> Result<(), Box<dyn Error>> {
    let mut fragmet: Option<Frame> = None;
    let mut header_buf = [0u8; 14];
    loop {
        let mut frame = match read_header(&mut soc_rx, &mut header_buf).await? {
            Some(f) => f,
            None => break,
        };
        read_payload(&mut soc_rx, &mut frame).await?;

        frame.validate(deflate_supported, fragmet.is_some())?;
        match frame.fragmet() {
            Fragmet::Start => {
                fragmet = Some(frame);
                continue;
            }
            Fragmet::Middle => {
                let mut f = fragmet.unwrap();
                f.append(&frame);
                fragmet = Some(f);
                continue;
            }
            Fragmet::End => {
                let mut f = fragmet.unwrap();
                f.append(&frame);
                frame = f;
                fragmet = None;
            }
            Fragmet::None => (),
        }
        frame.validate_payload()?;

        debug!(log, "message" ;"kind" => frame.kind(), "payload_len" => frame.payload_len);
        let is_close = frame.is_close();
        handle_frame(frame, &mut raw_tx, &mut session_tx).await?;
        if is_close {
            break;
        }
    }
    Ok(())
}

async fn handle_frame(
    frame: Frame,
    raw_tx: &mut Sender<Vec<u8>>,
    session_tx: &mut Sender<Msg>,
) -> Result<(), Box<dyn Error>> {
    match frame.opcode {
        TEXT | BINARY => session_tx.send(frame2_msg(frame)).await?,
        CLOSE => raw_tx.send(close_message()).await?,
        PING => raw_tx.send(frame.to_pong()).await?,
        PONG => (),
        _ => return Err(format!("reserved ws frame opcode {}", frame.opcode).into()),
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
    text_payload: String,
}

// data frame types
const CONTINUATION: u8 = 0;
const TEXT: u8 = 1;
const BINARY: u8 = 2;
// constrol frame types
const CLOSE: u8 = 8;
const PING: u8 = 9;
const PONG: u8 = 10;

enum Fragmet {
    Start,
    Middle,
    End,
    None,
}

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
            text_payload: String::new(),
        }
    }

    // length of the rest of the header after first two bytes
    fn header_len(&self) -> Option<usize> {
        if !self.mask && self.payload_len < 126 {
            return None;
        }
        let mut n: usize = if self.mask { 4 } else { 0 };
        if self.payload_len >= 126 {
            n += 2;
            if self.payload_len == 127 {
                n += 6;
            }
        }
        Some(n)
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

    fn set_payload(&mut self, mut payload: Vec<u8>) {
        if self.mask {
            // unmask payload
            // loop through the octets of ENCODED and XOR the octet with the (i modulo 4)th octet of MASK
            // ref: https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API/Writing_WebSocket_servers
            for (i, b) in payload.iter_mut().enumerate() {
                *b = *b ^ self.masking_key[i % 4];
            }
        }
        self.payload = payload;
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

    fn validate(&self, deflate_supported: bool, in_continuation: bool) -> Result<(), String> {
        if self.is_control_frame() && self.payload_len > 125 {
            return Err(format!("too log control frame len: {}", self.payload_len).into());
        }
        if self.is_control_frame() && !self.fin {
            return Err(format!("fragmented control frame").into());
        }
        if !self.is_control_frame() {
            if !in_continuation && self.opcode == CONTINUATION {
                return Err(format!("wrong continuation frame").into());
            }
            if in_continuation && self.opcode != CONTINUATION {
                return Err(format!("wrong continuation frame").into());
            }
        }
        if !self.is_rsv_ok(deflate_supported) {
            return Err(format!("wrong rsv").into());
        }
        Ok(())
    }

    fn validate_payload(&mut self) -> Result<(), String> {
        self.inflate()?;
        if self.opcode != TEXT {
            return Ok(());
        }
        match str::from_utf8(&self.payload) {
            Ok(s) => self.text_payload = s.to_owned(),
            Err(e) => return Err(format!("payload is not valid utf-8 string error: {}", e)),
        }
        Ok(())
    }

    fn inflate(&mut self) -> Result<(), String> {
        if self.rsv1 && self.payload_len > 0 {
            match inflate_bytes(&self.payload) {
                Ok(p) => self.payload = p,
                Err(e) => return Err(format!("failed to inflate payload error: {}", e)),
            }
        }
        Ok(())
    }

    fn fragmet(&self) -> Fragmet {
        if !self.fin && self.is_data_frame() {
            return Fragmet::Start;
        }
        if !self.fin && self.opcode == CONTINUATION {
            return Fragmet::Middle;
        }
        if self.fin && self.opcode == CONTINUATION {
            return Fragmet::End;
        }
        Fragmet::None
    }

    fn is_close(&self) -> bool {
        self.opcode == CLOSE
    }

    fn to_pong(&self) -> Vec<u8> {
        create_message(PONG, &self.payload)
    }

    fn append(&mut self, other: &Frame) -> &Frame {
        self.payload_len = self.payload_len + other.payload_len;
        self.payload.extend_from_slice(&other.payload);
        self
    }

    fn kind(&self) -> &str {
        match self.opcode {
            TEXT => "text",
            BINARY => "binary",
            CLOSE => "close",
            PING => "ping",
            PONG => "pong",
            _ => "reserved",
        }
    }
}

fn frame2_msg(frame: Frame) -> Msg {
    Msg {
        is_binary: frame.opcode == BINARY,
        binary: frame.payload,
        text: frame.text_payload,
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
    }
    #[test]
    fn frame_inflate() {
        let mut f = Frame::new(0, 0);
        f.rsv1 = true;
        f.rsv = 4;
        f.payload_len = 7;
        f.payload = vec![0xf2, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00];
        //f.payload = vec![0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x00, 0x00, 0xff, 0xff];
        assert_eq!(true, f.inflate().is_ok());
        assert_eq!("Hello", f.text_payload);
    }
}
