use super::{Error, Msg};
use crate::http;
use inflate::inflate_bytes;
use std::fmt;
use std::str;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf};
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};

pub async fn handle(hu: http::Upgrade, log: slog::Logger) -> (Receiver<Msg>, Sender<Msg>) {
    trace!(log, "open");
    let (soc_rx, mut soc_tx) = io::split(hu.stream);
    let (raw_tx, mut raw_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel(16);
    let (session_tx, session_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(16);
    let (conn_tx, mut conn_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(16);

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
    });

    // reads tcp socket, parse incoming stream as ws frames and then into application messages
    // soc_rx -> raw_tx       - for control frames
    //        -> session_tx   - for data frames
    let mut conn = Conn::new(hu.deflate_supported, soc_rx, raw_tx.clone(), session_tx, log.clone());
    spawn(async move {
        if let Err(e) = conn.read().await {
            error!(conn.log, "{}", e);
        }
    });

    // transforms application Msg to raw
    // conn_rx -> raw_tx
    let l = log.clone();
    let mut raw_session_tx = raw_tx.clone();
    spawn(async move {
        while let Some(m) = conn_rx.recv().await {
            if let Err(e) = raw_session_tx.send(m.as_raw()).await {
                error!(l, "{}", e);
                break;
            }
        }
    });

    (session_rx, conn_tx)
}

impl Msg {
    fn as_raw(&self) -> Vec<u8> {
        match self {
            Msg::Binary(b) => binary_frame(&b),
            Msg::Text(t) => text_frame(&t),
            //Msg::Close => close_message(),
        }
    }
}

struct Conn {
    deflate_supported: bool,
    soc_rx: ReadHalf<TcpStream>,
    raw_tx: Sender<Vec<u8>>,
    session_tx: Sender<Msg>,
    log: slog::Logger,
    header_buf: [u8; 14],
}

impl Conn {
    fn new(
        deflate_supported: bool,
        soc_rx: ReadHalf<TcpStream>,
        raw_tx: Sender<Vec<u8>>,
        session_tx: Sender<Msg>,
        log: slog::Logger,
    ) -> Conn {
        Conn {
            deflate_supported: deflate_supported,
            soc_rx: soc_rx,
            raw_tx: raw_tx,
            session_tx: session_tx,
            log: log,
            header_buf: [0u8; 14],
        }
    }

    async fn read_payload(&mut self, frame: &mut Frame) -> Result<(), Error> {
        let l = frame.payload_len as usize;
        if l > 0 {
            let mut buf = vec![0u8; l];
            self.soc_rx.read_exact(&mut buf).await?;
            frame.set_payload(buf.to_vec());
        }
        Ok(())
    }

    async fn read_header(&mut self) -> Result<Option<Frame>, Error> {
        if let Err(e) = self.soc_rx.read_exact(&mut self.header_buf[0..2]).await {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(None);
            }
            return Err(e.into());
        }
        let mut frame = Frame::new(self.header_buf[0], self.header_buf[1]);

        if let Some(l) = frame.header_len() {
            let b = &mut self.header_buf[2..l + 2];
            self.soc_rx.read_exact(b).await?;
            frame.set_header(b);
        }
        Ok(Some(frame))
    }

    async fn read(&mut self) -> Result<(), Error> {
        let mut fragment: Option<Frame> = None;
        loop {
            let mut frame = match self.read_header().await? {
                Some(f) => f,
                None => break,
            };
            self.read_payload(&mut frame).await?;

            frame.validate(self.deflate_supported, fragment.is_some())?;
            if frame.is_fragment() {
                let (new_frame, new_fragment) = frame.to_fragment(fragment);
                fragment = new_fragment;
                match new_frame {
                    Some(f) => frame = f,
                    None => continue, // current frame is fragment, wait for more
                }
            }
            frame.validate_payload()?;

            trace!(self.log, "message" ;"opcode" =>  frame.opcode.desc(), "len" => frame.payload_len);
            let is_close = frame.is_close();
            self.handle_frame(frame).await?;
            if is_close {
                break;
            }
        }
        Ok(())
    }

    async fn handle_frame(&mut self, frame: Frame) -> Result<(), Error> {
        match frame.opcode.value() {
            TEXT | BINARY => self.session_tx.send(frame2_msg(frame)).await?,
            CLOSE => self.raw_tx.send(close_frame()).await?,
            PING => self.raw_tx.send(frame.to_pong()).await?,
            PONG | _ => (),
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Frame {
    fin: bool,
    rsv1: bool,
    rsv: u8,
    mask: bool,
    opcode: Opcode,
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

#[derive(Debug)]
struct Opcode(u8);

impl Opcode {
    fn new(opcode: u8) -> Self {
        Self(opcode)
    }
    fn value(&self) -> u8 {
        self.0
    }
    fn valid(&self) -> bool {
        self.data() || self.control() || self.continuation()
    }
    fn data(&self) -> bool {
        self.0 == TEXT || self.0 == BINARY
    }
    fn control(&self) -> bool {
        self.0 == CLOSE || self.0 == PING || self.0 == PONG
    }
    fn continuation(&self) -> bool {
        self.0 == CONTINUATION
    }
    fn text(&self) -> bool {
        self.0 == TEXT
    }
    fn binary(&self) -> bool {
        self.0 == BINARY
    }
    fn close(&self) -> bool {
        self.0 == CLOSE
    }
    // fn ping(&self) -> bool {
    //     self.0 == 9
    // }
    // fn pong(&self) -> bool {
    //     self.0 == 10
    //}
    fn desc(&self) -> &str {
        match self.0 {
            CONTINUATION => "continuation",
            TEXT => "text",
            BINARY => "binary",
            CLOSE => "close",
            PING => "ping",
            PONG => "pong",
            _ => "reserved",
        }
    }
}

impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.desc())
    }
}

enum Fragmet {
    Start,
    Middle,
    End,
    None,
}

impl Frame {
    fn new(byte1: u8, byte2: u8) -> Frame {
        let opcode = byte1 & 0b0000_1111u8;
        Frame {
            fin: byte1 & 0b1000_0000u8 != 0,
            rsv1: byte1 & 0b0100_0000u8 != 0,
            rsv: (byte1 & 0b0111_0000u8) >> 4,
            opcode: Opcode::new(opcode),
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

    fn is_rsv_ok(&self, deflate_supported: bool) -> bool {
        if deflate_supported {
            return self.rsv == 0 || self.rsv == 4;
        }
        // rsv must be 0, when no extension defining RSV meaning has been negotiated
        self.rsv == 0
    }

    fn validate(&self, deflate_supported: bool, in_continuation: bool) -> Result<(), Error> {
        if !self.opcode.valid() {
            return Err(Error::WrongHeader(format!("reserved opcode {}", self.opcode.value())));
        }
        if self.opcode.control() {
            // control frames must be short, payload <= 125 bytes
            // can't be split into fragments
            if self.payload_len > 125 {
                return Err(Error::WrongHeader(format!(
                    "too long control frame {} > 125",
                    self.payload_len
                )));
            }
            if !self.fin {
                return Err(Error::WrongHeader("fragmented control frame".to_owned()));
            }
        } else {
            // continuation (waiting for more fragmets) frames must be in order start/middle.../end
            if !in_continuation && self.opcode.continuation() {
                return Err(Error::WrongHeader("not in continuation".to_owned()));
            }
            if in_continuation && !self.opcode.continuation() {
                return Err(Error::WrongHeader("fin frame during continuation".to_owned()));
            }
        }
        if !self.is_rsv_ok(deflate_supported) {
            // only bit 1 of rsv is currently used
            return Err(Error::WrongHeader("wrong rsv".to_owned()));
        }
        Ok(())
    }

    fn validate_payload(&mut self) -> Result<(), Error> {
        self.inflate()?;
        if !self.opcode.text() {
            return Ok(());
        }
        match str::from_utf8(&self.payload) {
            Ok(s) => self.text_payload = s.to_owned(),
            Err(e) => return Err(Error::TextPayloadNotValidUTF8(e)),
        }
        Ok(())
    }

    fn inflate(&mut self) -> Result<(), Error> {
        if self.rsv1 && self.payload_len > 0 {
            match inflate_bytes(&self.payload) {
                Ok(p) => self.payload = p,
                Err(e) => return Err(Error::InflateFailed(e)),
            }
        }
        Ok(())
    }

    fn fragmet(&self) -> Fragmet {
        if !self.fin && self.opcode.data() {
            return Fragmet::Start;
        }
        if !self.fin && self.opcode.continuation() {
            return Fragmet::Middle;
        }
        if self.fin && self.opcode.continuation() {
            return Fragmet::End;
        }
        Fragmet::None
    }
    fn is_fragment(&self) -> bool {
        !(self.fin && !self.opcode.continuation())
    }

    // if frame is part of the fragmented message it is appended to the current fragment
    // returns frame, and fragment
    // if frame is None it is not completed
    fn to_fragment(self, fragment: Option<Frame>) -> (Option<Frame>, Option<Frame>) {
        match self.fragmet() {
            Fragmet::Start => (None, Some(self)),
            Fragmet::Middle => {
                let mut f = fragment.unwrap();
                f.append(&self);
                (None, Some(f))
            }
            Fragmet::End => {
                let mut f = fragment.unwrap();
                f.append(&self);
                (Some(f), None)
            }
            Fragmet::None => (Some(self), fragment),
        }
    }

    fn is_close(&self) -> bool {
        self.opcode.close()
    }

    fn to_pong(&self) -> Vec<u8> {
        create_frame(PONG, &self.payload)
    }

    fn append(&mut self, other: &Frame) -> &Frame {
        self.payload_len = self.payload_len + other.payload_len;
        self.payload.extend_from_slice(&other.payload);
        self
    }
}

fn frame2_msg(frame: Frame) -> Msg {
    if frame.opcode.binary() {
        return Msg::Binary(frame.payload);
    }
    Msg::Text(frame.text_payload)
}

fn close_frame() -> Vec<u8> {
    vec![0b1000_1000u8, 0b0000_0000u8]
}

fn text_frame(text: &str) -> Vec<u8> {
    create_frame(TEXT, text.as_bytes())
}

fn binary_frame(data: &[u8]) -> Vec<u8> {
    create_frame(BINARY, data)
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
fn create_frame(opcode: u8, body: &[u8]) -> Vec<u8> {
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
        let buf = text_frame("abc");
        assert_eq!(5, buf.len());
        assert_eq!([0x81, 0x03, 0x61, 0x62, 0x63], buf[0..]);

        let buf = text_frame(
            "The length of the Payload data, in bytes: if 0-125, that is the payload length.  
        If 126, the following 2 bytes interpreted as a 16-bit unsigned integer are the payload length.",
        );
        assert_eq!(188, buf.len());
        assert_eq!([0x81, 0x7e, 0x00, 184], buf[0..4]);
        assert_eq!(184, buf[3]); // 175 is body length

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
        f.opcode = Opcode::new(1);
        f.payload_len = 7;
        f.payload = vec![0xf2, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00];
        assert_eq!(true, f.validate_payload().is_ok());
        assert_eq!("Hello", f.text_payload);
    }
}
