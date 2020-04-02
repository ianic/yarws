use std::str;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::prelude::*;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::http;

pub async fn handle(stream: TcpStream) -> io::Result<()> {
    http::upgrade(stream).await
}

pub async fn handle_ws(stream: TcpStream) -> io::Result<()> {
    println!("ws open");
    let (input, output) = io::split(stream);
    let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel(16);

    spawn(async move {
        if let Err(e) = write(output, rx).await {
            println!("ws_write error {:?}", e);
        }
    });
    spawn(async move {
        if let Err(e) = read(input, tx).await {
            println!("ws_read error {:?}", e);
        }
    });
    println!("ws spawned");
    Ok(())
}

async fn read(mut input: ReadHalf<TcpStream>, mut rx: Sender<Vec<u8>>) -> io::Result<()> {
    loop {
        let mut buf = [0u8; 2];
        input.read_exact(&mut buf).await?;

        let mut h = Header::new(buf[0], buf[1]);
        let rn = h.read_next() as usize;
        if rn > 0 {
            let mut buf = vec![0u8; rn];
            input.read_exact(&mut buf).await?;
            h.set_header(&buf);
        }

        let rn = h.payload_len as usize;
        if rn > 0 {
            let mut buf = vec![0u8; rn];
            input.read_exact(&mut buf).await?;
            h.set_payload(&buf);
        }

        match h.kind() {
            FrameKind::Text => println!("ws body {} bytes, as str: {}", rn, h.payload_str()),
            FrameKind::Binary => println!("ws body is binary frame of size {}", h.payload_len),
            FrameKind::Close => {
                println!("ws close");
                if let Err(_) = rx.send(close_frame()).await {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "failed to send close",
                    ));
                }
                break;
            }
            FrameKind::Ping => {
                println!("ws ping");
                if let Err(_) = rx.send(h.to_pong()).await {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "failed to send pong",
                    ));
                }
            }
            FrameKind::Pong => println!("ws pong"),
            FrameKind::Continuation => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ws continuation frame not supported",
                ));
            }
            FrameKind::Reserved(opcode) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("reserved ws frame opcode {}", opcode),
                ));
            }
        }

        if let Err(_) = rx.send(pero_frame()).await {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "failed to send a frame",
            ));
        }
    }

    Ok(())
}

async fn write(mut output: WriteHalf<TcpStream>, mut rx: Receiver<Vec<u8>>) -> io::Result<()> {
    while let Some(v) = rx.recv().await {
        let n = output.write(&v).await?;
        println!("ws written {} bytes", n);
        output.flush().await?;
    }
    println!("write half closed");
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
