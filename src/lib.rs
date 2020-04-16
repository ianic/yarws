use slog::Logger;
use std::str;
use tokio;
use tokio::io;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::spawn;
use tokio::stream::StreamExt;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use url::Url;

#[macro_use]
extern crate slog;
#[macro_use]
extern crate failure;

#[cfg(test)]
#[macro_use]
extern crate hex_literal;

mod http;
pub mod log;
mod ws;

pub struct Socket {
    pub no: usize,
    tx: Sender<ws::Msg>,
    rx: Receiver<ws::Msg>,
}

impl Socket {
    pub async fn recv(&mut self) -> Option<Msg> {
        loop {
            match self.rx.recv().await {
                None => return None, // channel exhausted
                Some(m) => {
                    let opt_msg = m.into_msg();
                    match opt_msg {
                        None => continue, // skip control message type
                        _ => return opt_msg,
                    }
                }
            }
        }
    }

    pub async fn recv_text(&mut self) -> Option<String> {
        loop {
            match self.rx.recv().await {
                None => return None,
                Some(m) => match m {
                    ws::Msg::Text(text) => return Some(text),
                    _ => continue, // ignore other type of the messages
                },
            }
        }
    }

    pub async fn must_recv_text(&mut self) -> Result<String, Error> {
        match self.recv_text().await {
            None => Err(Error::SocketClosed),
            Some(v) => Ok(v),
        }
    }

    pub async fn send(&mut self, msg: Msg) -> Result<(), Error> {
        self.tx.send(msg.into_ws_msg()).await?;
        Ok(())
    }

    pub async fn send_text(&mut self, text: &str) -> Result<(), Error> {
        self.tx.send(ws::Msg::Text(text.to_owned())).await?;
        Ok(())
    }

    pub async fn into_text_chan(self) -> (Sender<String>, Receiver<String>) {
        let (tx, mut i_rx): (Sender<String>, Receiver<String>) = mpsc::channel(1);
        let (mut i_tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel(1);

        let mut ws_rx = self.rx;
        spawn(async move {
            while let Some(ws_msg) = ws_rx.recv().await {
                if let Some(msg) = ws_msg.into_msg() {
                    match msg {
                        Msg::Text(text) => {
                            if let Err(_) = i_tx.send(text).await {
                                break;
                            }
                        }
                        _ => (),
                    }
                }
            }
        });

        let mut ws_tx = self.tx;
        spawn(async move {
            while let Some(text) = i_rx.recv().await {
                if let Err(_) = ws_tx.send(ws::Msg::Text(text)).await {
                    break;
                }
            }
        });

        (tx, rx)
    }
}

pub enum Msg {
    Text(String),
    Binary(Vec<u8>),
}

impl Msg {
    fn into_ws_msg(self) -> ws::Msg {
        match self {
            Msg::Text(text) => ws::Msg::Text(text),
            Msg::Binary(vec) => ws::Msg::Binary(vec),
        }
    }
}

pub struct Server {
    rx: Receiver<Socket>,
}

impl Server {
    pub async fn bind<L: Into<Option<slog::Logger>>>(addr: &str, log: L) -> Result<Self, Error> {
        let log = log.into().unwrap_or(log::null());
        let listener = TcpListener::bind(addr).await?;
        Ok(Self {
            rx: Server::listen(listener, log).await,
        })
    }

    pub async fn accept(&mut self) -> Option<Socket> {
        self.rx.recv().await
    }

    async fn listen(mut listener: TcpListener, log: Logger) -> Receiver<Socket> {
        let (socket_tx, socket_rx): (Sender<Socket>, Receiver<Socket>) = mpsc::channel(1);

        let mut conn_no = 0;
        spawn(async move {
            let mut incoming = listener.incoming();
            while let Some(conn) = incoming.next().await {
                match conn {
                    Ok(stream) => {
                        conn_no += 1;
                        let log = log.new(o!("conn" => conn_no));
                        let stx = socket_tx.clone();
                        spawn(async move { handle_stream(stream, conn_no, stx, log).await });
                    }
                    Err(e) => error!(log, "accept error: {}", e),
                }
            }
        });

        socket_rx
    }
}

async fn handle_stream(stream: TcpStream, no: usize, mut sockets_tx: Sender<Socket>, log: Logger) {
    match http::upgrade(stream).await {
        Ok(u) => {
            let (rx, tx) = ws::handle(u, log.clone()).await;
            let s = Socket { no: no, rx: rx, tx: tx };
            if let Err(e) = sockets_tx.send(s).await {
                error!(log, "unable to open socket: {}", e);
            }
        }
        Err(e) => error!(log, "upgrade error: {}", e),
    }
}

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "invalid upgrade request")]
    InvalidUpgradeRequest,
    #[fail(display = "IO error: {}", error)]
    IoError { error: io::Error },
    #[fail(display = "fail to send message upstream: {}", error)]
    UpstreamError { error: mpsc::error::SendError<ws::Msg> },
    #[fail(display = "fail to send frame: {}", error)]
    ConnError { error: mpsc::error::SendError<Vec<u8>> },
    #[fail(display = "wrong header: {}", _0)]
    WrongHeader(String),
    #[fail(display = "inflate failed: {}", _0)]
    InflateFailed(String),
    #[fail(display = "text payload not a valid utf-8 string: {}", _0)]
    TextPayloadNotValidUTF8(std::str::Utf8Error),
    #[fail(display = "failed to parse url: {} error: {}", url, error)]
    UrlParseError { url: String, error: url::ParseError },
    #[fail(display = "socket closed")]
    SocketClosed,
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IoError { error: e }
    }
}
impl From<mpsc::error::SendError<ws::Msg>> for Error {
    fn from(e: mpsc::error::SendError<ws::Msg>) -> Self {
        Error::UpstreamError { error: e }
    }
}
impl From<mpsc::error::SendError<Vec<u8>>> for Error {
    fn from(e: mpsc::error::SendError<Vec<u8>>) -> Self {
        Error::ConnError { error: e }
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(e: std::str::Utf8Error) -> Self {
        Error::TextPayloadNotValidUTF8(e)
    }
}

pub async fn connect<L: Into<Option<slog::Logger>>>(url: &str, log: L) -> Result<Socket, Error> {
    let log = log.into().unwrap_or(log::null());
    let (addr, path) = parse_url(url)?;
    let stream = TcpStream::connect(&addr).await?;
    let upgrade = http::connect(stream, &addr, &path).await?;
    let (rx, tx) = ws::handle(upgrade, log.clone()).await;
    Ok(Socket { no: 1, rx: rx, tx: tx })
}

fn parse_url(u: &str) -> Result<(String, String), Error> {
    let url = match match Url::parse(u) {
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let url = "ws://".to_owned() + u;
            Url::parse(&url)
        }
        other => other,
    } {
        Err(e) => Err(Error::UrlParseError {
            url: u.to_owned(),
            error: e,
        }),
        Ok(v) => Ok(v),
    }?;
    let host = url.host_str().unwrap_or("");
    let addr = format!("{}:{}", host, url.port_or_known_default().unwrap_or(0));
    let path = match url.query() {
        Some(q) => format!("{}?{}", url.path(), q),
        None => url.path().to_owned(),
    };
    Ok((addr, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_url() {
        let (addr, path) = parse_url("ws://localhost:9001/path?pero=zdero").unwrap();
        assert_eq!("localhost:9001", addr);
        assert_eq!("/path?pero=zdero", path);

        let (addr, path) = parse_url("ws://localhost:9001/path").unwrap();
        assert_eq!("localhost:9001", addr);
        assert_eq!("/path", path);

        let (addr, path) = parse_url("ws://localhost/path").unwrap();
        assert_eq!("localhost:80", addr);
        assert_eq!("/path", path);

        let (addr, path) = parse_url("localhost/path").unwrap();
        assert_eq!("localhost:80", addr);
        assert_eq!("/path", path);

        let (addr, path) = parse_url("pero://localhost/path").unwrap();
        assert_eq!("localhost:0", addr);
        assert_eq!("/path", path);
    }
}
