use super::{Error, Msg};
use crate::http;
use inflate::inflate_bytes;
use rand::Rng;
use slog::Logger;
use std::fmt;
use std::str;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};

pub async fn handle(upgrade: http::Upgrade, log: Logger) -> (Receiver<Msg>, Sender<Msg>) {
    trace!(log, "open");

    let mask_frames = upgrade.client;
    let deflate_supported = upgrade.deflate_supported;

    // rx receive end
    // tx transmit end
    // stream is tcp connection
    // Reader is transforming form raw tcp stream to the valid websocket frames
    // socket is websocet implementation passed downstream
    // Writer is writing to the outbound tcp stream
    let (stream_rx, stream_tx) = io::split(upgrade.stream);
    let (reader_tx, socket_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(16);
    let (socket_tx, writer_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(16);

    let control_tx = Writer::new(stream_tx, writer_rx, mask_frames, log.clone()).spawn();

    Reader::new(deflate_supported, mask_frames, stream_rx, control_tx, reader_tx, log).spawn();

    (socket_rx, socket_tx)
}

// Writer writes raw data to the outbound tcp stream
struct Writer {
    stream_tx: WriteHalf<TcpStream>,
    writer_rx: Receiver<Msg>,
    mask_frames: bool,
    log: Logger,
}

impl Writer {
    fn new(stream_tx: WriteHalf<TcpStream>, writer_rx: Receiver<Msg>, mask_frames: bool, log: Logger) -> Self {
        Self {
            stream_tx: stream_tx,
            writer_rx: writer_rx,
            mask_frames: mask_frames,
            log: log,
        }
    }

    fn spawn(self) -> Sender<Vec<u8>> {
        let (raw_tx, raw_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel(16);
        let control_tx = raw_tx.clone();

        // out loop writes raw data to the tcp stream
        Writer::out_loop(raw_rx, self.stream_tx, self.log.clone());
        // inner loop transforms Msg from upstream application to the raw data
        Writer::inner_loop(self.writer_rx, raw_tx, self.mask_frames, self.log);

        control_tx // reader controll messages are passed to the out loop through this channel
    }

    fn out_loop(mut raw_rx: Receiver<Vec<u8>>, mut stream_tx: WriteHalf<TcpStream>, log: Logger) {
        spawn(async move {
            while let Some(v) = raw_rx.recv().await {
                if let Err(e) = stream_tx.write(&v).await {
                    error!(log, "{}", e);
                    break;
                }
            }
        });
    }

    fn inner_loop(mut writer_rx: Receiver<Msg>, mut raw_tx: Sender<Vec<u8>>, mask_frames: bool, log: Logger) {
        spawn(async move {
            while let Some(m) = writer_rx.recv().await {
                if let Err(e) = raw_tx.send(m.as_raw(mask_frames)).await {
                    error!(log, "{}", e);
                    break;
                }
            }
        });
    }
}

impl Msg {
    fn as_raw(&self, client: bool) -> Vec<u8> {
        let w = FrameWriter::new(client);
        match self {
            Msg::Binary(b) => w.binary(b.to_vec()),
            Msg::Text(t) => w.text(t.to_string()),
        }
    }
}

struct Reader {
    deflate_supported: bool,
    mask_frames: bool,
    stream_rx: ReadHalf<TcpStream>,
    control_tx: Sender<Vec<u8>>,
    reader_tx: Sender<Msg>,
    log: slog::Logger,
    header_buf: [u8; 14],
}

impl Reader {
    fn new(
        deflate_supported: bool,
        mask_frames: bool,
        stream_rx: ReadHalf<TcpStream>,
        control_tx: Sender<Vec<u8>>,
        reader_tx: Sender<Msg>,
        log: slog::Logger,
    ) -> Reader {
        Reader {
            deflate_supported: deflate_supported,
            mask_frames: mask_frames,
            stream_rx: stream_rx,
            control_tx: control_tx,
            reader_tx: reader_tx,
            log: log,
            header_buf: [0u8; 14],
        }
    }

    fn spawn(mut self) {
        spawn(async move {
            if let Err(e) = self.read().await {
                error!(self.log, "{}", e);
            }
        });
    }

    async fn read_payload(&mut self, frame: &mut Frame) -> Result<(), Error> {
        let l = frame.payload_len as usize;
        if l > 0 {
            let mut buf = vec![0u8; l];
            self.stream_rx.read_exact(&mut buf).await?;
            frame.set_payload(buf.to_vec());
        }
        Ok(())
    }

    async fn read_header(&mut self) -> Result<Option<Frame>, Error> {
        if let Err(e) = self.stream_rx.read_exact(&mut self.header_buf[0..2]).await {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(None);
            }
            return Err(e.into());
        }
        let mut frame = Frame::new(self.header_buf[0], self.header_buf[1]);

        if let Some(l) = frame.header_len() {
            let b = &mut self.header_buf[2..l + 2];
            self.stream_rx.read_exact(b).await?;
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
                trace!(self.log, "fragment" ;"opcode" =>  frame.opcode.desc(), "len" => frame.payload_len);
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
        trace!(self.log, "read half closed");
        Ok(())
    }

    async fn handle_frame(&mut self, frame: Frame) -> Result<(), Error> {
        match frame.opcode.value() {
            TEXT | BINARY => self.reader_tx.send(frame.to_msg()).await?,
            CLOSE => self.control_tx.send(frame.to_close(self.mask_frames)).await?,
            PING => self.control_tx.send(frame.to_pong(self.mask_frames)).await?,
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
// control frame types
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

enum Fragment {
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
        let mask_start = if self.mask { buf.len() - 4 } else { buf.len() };
        if mask_start == 8 {
            let bytes: [u8; 8] = [buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]];
            self.payload_len = u64::from_be_bytes(bytes);
        }
        if mask_start == 2 {
            let bytes: [u8; 2] = [buf[0], buf[1]];
            self.payload_len = u16::from_be_bytes(bytes) as u64;
        }
        if self.mask {
            for i in 0..4 {
                self.masking_key[i] = buf[mask_start + i];
            }
        }
    }

    fn set_payload(&mut self, mut payload: Vec<u8>) {
        if self.mask {
            payload = mask(payload, self.masking_key);
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
            // continuation (waiting for more fragments) frames must be in order start/middle.../end
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
        self.text_payload = str::from_utf8(&self.payload)?.to_owned();
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

    fn fragment(&self) -> Fragment {
        if !self.fin && self.opcode.data() {
            return Fragment::Start;
        }
        if !self.fin && self.opcode.continuation() {
            return Fragment::Middle;
        }
        if self.fin && self.opcode.continuation() {
            return Fragment::End;
        }
        Fragment::None
    }
    fn is_fragment(&self) -> bool {
        !(self.fin && !self.opcode.continuation())
    }

    // if frame is part of the fragmented message it is appended to the current fragment
    // returns frame, and fragment
    // if frame is None it is not completed
    fn to_fragment(self, fragment: Option<Frame>) -> (Option<Frame>, Option<Frame>) {
        match self.fragment() {
            Fragment::Start => (None, Some(self)),
            Fragment::Middle => {
                let mut f = fragment.unwrap();
                f.append(&self);
                (None, Some(f))
            }
            Fragment::End => {
                let mut f = fragment.unwrap();
                f.append(&self);
                (Some(f), None)
            }
            Fragment::None => (Some(self), fragment),
        }
    }

    fn is_close(&self) -> bool {
        self.opcode.close()
    }

    fn to_pong(self, mask_frames: bool) -> Vec<u8> {
        FrameWriter::new(mask_frames).pong(self.payload)
        //FrameWriter::control(PONG, self.payload)
    }

    fn to_close(self, mask_frames: bool) -> Vec<u8> {
        let mut payload: Vec<u8> = Vec::new();
        if self.payload_len == 2 {
            let bytes: [u8; 2] = [self.payload[0], self.payload[1]];
            let status = u16::from_be_bytes(bytes);
            let status = match status {
                1000 | 1001 | 1002 | 1003 | 1007 | 1008 | 1009 | 1010 | 1011 => status, // valid status code, reply with that code
                _ => 0, // for all other reply with close frame without payload
            };
            if status > 0 {
                payload.extend_from_slice(&status.to_be_bytes());
            }
        }
        FrameWriter::new(mask_frames).close(payload)
    }

    fn append(&mut self, other: &Frame) -> &Frame {
        self.payload_len = self.payload_len + other.payload_len;
        self.payload.extend_from_slice(&other.payload);
        self
    }

    fn to_msg(self) -> Msg {
        if self.opcode.binary() {
            return Msg::Binary(self.payload);
        }
        Msg::Text(self.text_payload)
    }
}

//Converts masked data into unmasked data, or vice versa.
//The same algorithm applies regardless of the direction of the translation,
//e.g., the same steps are applied to ask the data as to unmask the data.
fn mask(mut payload: Vec<u8>, key: [u8; 4]) -> Vec<u8> {
    // loop through the octets of ENCODED and XOR the octet with the (i modulo 4)th octet of MASK
    // ref: https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API/Writing_WebSocket_servers
    for (i, b) in payload.iter_mut().enumerate() {
        *b = *b ^ key[i % 4];
    }
    payload
}

struct FrameWriter {
    mask: bool,
}

impl FrameWriter {
    fn new(mask: bool) -> Self {
        Self { mask: mask }
    }

    fn pong(&self, payload: Vec<u8>) -> Vec<u8> {
        self.build(PONG, payload)
    }

    fn close(&self, payload: Vec<u8>) -> Vec<u8> {
        self.build(CLOSE, payload)
    }

    fn binary(&self, payload: Vec<u8>) -> Vec<u8> {
        self.build(BINARY, payload)
    }

    fn text(&self, payload: String) -> Vec<u8> {
        self.build(TEXT, payload.into_bytes())
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
    fn build(&self, opcode: u8, mut payload: Vec<u8>) -> Vec<u8> {
        let mut buf = vec![0b1000_0000u8 + opcode];

        // add payload length
        let l = payload.len();
        if l < 126 {
            buf.push(l as u8);
        } else if payload.len() < 65536 {
            buf.push(126u8);
            let l = l as u16;
            buf.extend_from_slice(&l.to_be_bytes());
        } else {
            buf.push(127u8);
            let l = l as u64;
            buf.extend_from_slice(&l.to_be_bytes());
        }
        if self.mask {
            buf[1] = buf[1] | 0b1000_0000u8; // set masking bit
            let masking_key = rand::thread_rng().gen::<[u8; 4]>(); // create key
            buf.extend_from_slice(&masking_key); // write key to msg
            payload = mask(payload, masking_key) // mask payload
        }
        buf.extend_from_slice(payload.as_slice());
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::compress_to_vec;
    use miniz_oxide::inflate::decompress_to_vec;

    fn text_frame(text: &str) -> Vec<u8> {
        FrameWriter::new(false).text(text.to_owned())
    }

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
