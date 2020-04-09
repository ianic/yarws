use slog::Logger;
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
extern crate slog_async;
extern crate slog_term;
#[macro_use]
extern crate failure;

pub mod http;
pub mod ws;

pub struct Session {
    pub no: usize,
    pub tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
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

#[cfg(test)]
#[macro_use]
extern crate hex_literal;
