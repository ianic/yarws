use std::error::Error;
use std::str;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};

pub async fn handle(stream: TcpStream) {
    println!("ws open");
    let (input, output) = io::split(stream);
    let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel(16);

    spawn(async move {
        if let Err(e) = write(output, rx).await {
            println!("ws_write error {:?}", e);
        }
        println!("write half closed");
    });
    spawn(async move {
        if let Err(e) = read(input, tx).await {
            println!("ws_read error {:?}", e);
        }
        println!("read half closed");
    });
}

async fn read(mut input: ReadHalf<TcpStream>, mut rx: Sender<Vec<u8>>) -> Result<(), Box<dyn Error>> {
    loop {
        let mut buf = [0u8; 2];
        if let Err(e) = input.read_exact(&mut buf).await {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                break; //tcp closed without ws close handshake
            }
            return Err(Box::new(e));
        }

        let mut header = Header::new(buf[0], buf[1]);
        let rn = header.read_next() as usize;
        if rn > 0 {
            let mut buf = vec![0u8; rn];
            input.read_exact(&mut buf).await?;
            header.set_header(&buf);
        }

        let rn = header.payload_len as usize;
        if rn > 0 {
            let mut buf = vec![0u8; rn];
            input.read_exact(&mut buf).await?;
            header.set_payload(&buf);
        }

        match header.kind() {
            FrameKind::Text => println!("ws body {} bytes, as str: {}", rn, header.payload_str()),
            FrameKind::Binary => println!("ws body is binary frame of size {}", header.payload_len),
            FrameKind::Close => {
                println!("ws close");
                rx.send(close_frame()).await?;
            }
            FrameKind::Ping => {
                println!("ws ping");
                rx.send(header.to_pong()).await?;
            }
            FrameKind::Pong => println!("ws pong"),
            FrameKind::Continuation => {
                return Err("ws continuation frame not supported".into());
            }
            FrameKind::Reserved(opcode) => {
                return Err(format!("reserved ws frame opcode {}", opcode).into());
            }
        }

        rx.send(pero_frame()).await?
    }

    Ok(())
}

async fn write(mut output: WriteHalf<TcpStream>, mut rx: Receiver<Vec<u8>>) -> io::Result<()> {
    while let Some(v) = rx.recv().await {
        output.write(&v).await?;
    }
    Ok(())
}

struct Header {
    #[allow(dead_code)]
    fin: bool,
    #[allow(dead_code)]
    rsv1: bool,
    #[allow(dead_code)]
    rsv2: bool,
    #[allow(dead_code)]
    rsv3: bool,
    mask: bool,
    opcode: u8,
    payload_len: u64,
    masking_key: [u8; 4],
    payload: Vec<u8>,
}

enum FrameKind {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
    Reserved(u8),
}

impl Header {
    fn new(byte1: u8, byte2: u8) -> Header {
        Header {
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
    fn kind(&self) -> FrameKind {
        match self.opcode {
            0 => FrameKind::Continuation,
            1 => FrameKind::Text,
            2 => FrameKind::Binary,
            8 => FrameKind::Close,
            9 => FrameKind::Ping,
            0xa => FrameKind::Pong,
            _ => FrameKind::Reserved(self.opcode),
        }
    }
    fn to_pong(&self) -> Vec<u8> {
        vec![0b1000_1010u8, 0b00000000u8]
    }
}

fn pero_frame() -> Vec<u8> {
    let mut buf = vec![0b10000001u8, 0b00000100u8];
    for b in "pero".as_bytes() {
        buf.push(*b);
    }
    buf
}

fn close_frame() -> Vec<u8> {
    vec![0b1000_1000u8, 0b0000_0000u8]
}
