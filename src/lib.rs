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

pub struct Session {
    pub no: usize,
    pub tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
}

impl Session {
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

    pub async fn sessions(mut self) -> Receiver<Session> {
        let (session_tx, session_rx): (Sender<Session>, Receiver<Session>) = mpsc::channel(16);

        let mut conn_no = 0;
        spawn(async move {
            let mut incoming = self.listener.incoming();
            while let Some(conn) = incoming.next().await {
                match conn {
                    Ok(sock) => {
                        conn_no += 1;
                        let log = self.log.new(o!("conn" => conn_no));
                        let stx = session_tx.clone();
                        spawn(async move { handle_socket(sock, conn_no, stx, log).await });
                    }
                    Err(e) => error!(self.log, "accept error: {}", e),
                }
            }
        });

        session_rx
    }
}

async fn handle_socket(sock: TcpStream, no: usize, mut sessions_tx: Sender<Session>, log: Logger) {
    match http::upgrade(sock).await {
        Ok(u) => {
            let (rx, tx) = ws::handle(u, log.clone()).await;
            let s = Session { no: no, rx: rx, tx: tx };
            if let Err(e) = sessions_tx.send(s).await {
                error!(log, "unable to start session: {}", e);
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

pub async fn connect(addr: String, log: Logger) -> Result<Session, Error> {
    let stream = TcpStream::connect(&addr).await?;
    let upgrade = http::connect(stream, &addr).await?;
    let (rx, tx) = ws::handle(upgrade, log.clone()).await;
    Ok(Session { no: 1, rx: rx, tx: tx })
}

/*
TODOs
- clean exit
- how to use logger into library
- client messages should be masked
- znamo samo primati komprimirane poruke, ne i komprimirati
*/