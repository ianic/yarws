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
    pub tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
}

impl Socket {
    pub async fn send(&mut self, text: &str) -> Result<(), Error> {
        self.tx.send(Msg::Text(text.to_owned())).await?;
        Ok(())
    }
    pub async fn receive(&mut self) -> Result<Option<String>, Error> {
        match self.rx.recv().await {
            None => Ok(None),
            Some(m) => match m {
                Msg::Text(t) => Ok(Some(t)),
                Msg::Binary(buf) => match str::from_utf8(&buf) {
                    Ok(s) => Ok(Some(s.to_owned())),
                    Err(e) => return Err(Error::TextPayloadNotValidUTF8(e)),
                },
            },
        }
    }
}

pub struct Server {
    log: Logger,
    listener: TcpListener,
}

impl Server {
    pub async fn bind(addr: String, log: Logger) -> Result<Self, io::Error> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self {
            log: log,
            listener: listener,
        })
    }

    pub async fn listen(mut self) -> Receiver<Socket> {
        let (socket_tx, socket_rx): (Sender<Socket>, Receiver<Socket>) = mpsc::channel(16);

        let mut conn_no = 0;
        spawn(async move {
            let mut incoming = self.listener.incoming();
            while let Some(conn) = incoming.next().await {
                match conn {
                    Ok(stream) => {
                        conn_no += 1;
                        let log = self.log.new(o!("conn" => conn_no));
                        let stx = socket_tx.clone();
                        spawn(async move { handle_stream(stream, conn_no, stx, log).await });
                    }
                    Err(e) => error!(self.log, "accept error: {}", e),
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

#[derive(Debug)]
pub enum Msg {
    Binary(Vec<u8>),
    Text(String),
    //Close,
}

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "invalid upgrade request")]
    InvalidUpgradeRequest,
    #[fail(display = "IO error: {}", error)]
    IoError { error: io::Error },
    #[fail(display = "fail to send message upstream: {}", error)]
    UpstreamError { error: mpsc::error::SendError<Msg> },
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
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IoError { error: e }
    }
}
impl From<mpsc::error::SendError<Msg>> for Error {
    fn from(e: mpsc::error::SendError<Msg>) -> Self {
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

pub async fn connect(url: &str, log: Logger) -> Result<Socket, Error> {
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

/*
TODOs
- clean exit
- how to use logger into library
- znamo samo primati komprimirane poruke, ne i komprimirati
*/

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
