//! WebSocket protocol implementation based on [Tokio] runtime. For building
//! WebSocket server or client.
//!
//! yarws = Yet Another Rust WebSocket library
//!
//! Tls (wss:// enpoints) are supported in connect (since version 0.2.0).
//!
//! Lib is passing all [autobahn] tests. Including those for compressed
//! messages. Per message deflate is implemented for incoming messages. Lib can
//! receive compressed messages. Currently all outgoing messages are sent
//! uncompressed.
//!
//!
//! # Examples
//!
//! ## Server:
//! ```no-run
//! let mut srv = yarws::bind("127.0.0.1:9001", None).await?;
//! while let Some(socket) = srv.accept().await {
//!     tokio::spawn(async move {
//!         while let Some(msg) = socket.recv().await {
//!             socket.send(msg).await.unwrap();
//!         }
//!     }
//! }
//! ```
//! This is an example of echo server. We are replying with the same message on
//! each incoming message.  
//! First line starts listening for WebSocket connections on an ip:port.  
//! Each client is represented by [`Socket`] returned from [`accept`].  
//! For each client we are looping while messages arrive and replying with the
//! same message.  
//! For the complete echo server example please take a look at
//! [examples/echo_server.rs].
//!
//! ## Client:
//! ```no-run
//! let mut socket = yarws::connect("ws://127.0.0.1:9001", None).await?;
//! while let Some(msg) = socket.recv().await {
//!     socket.send(msg).await?;
//! }
//! ```
//! This is example of an echo client.  
//! [`connect`] method returns [`Socket`] which is used to send and receive
//! messages.  
//! Looping on recv returns each incoming message until socket is closed.  
//! Here in loop we reply with the same message.  
//! For the complete client example refer to [examples/client.rs].
//!
//!
//!
//! # Testing
//! Run client with external echo server.   
//! ```shell
//! cargo run --example client -- ws://echo.websocket.org
//! ```
//! Client will send few messages of different sizes and expect to get the same
//! in return.  
//! If everything went fine will finish without error.
//!
//! To run same client on our server. First start server:
//! ```shell
//! cargo run --example echo_server
//! ```
//! Then in other terminal run client:
//! ```shell
//! cargo run --example client
//! ```
//! If it is in trace log mode server will log type and size of every message it
//! receives.
//!
//! ## websocat test tool
//! You can use [websocat] to connect to the server and test communication.  
//! First start server:
//! ```shell
//! cargo run --example echo_server
//! ```
//! Then in other terminal run websocat:
//! ```shell
//! websocat -E --linemode-strip-newlines ws://127.0.0.1:9001
//! ```
//! Type you message press enter to send it and server will reply with the same
//! message.  
//! For more exciting server run it in with reverse flag:
//! ```shell
//! cargo run --bin echo_server -- --reverse
//! ```
//! and than use websocat to send text messages.
//!
//! # Autobahn tests
//! Ensure that you have [wstest] autobahn-testsuite test tool installed:
//! ```shell
//! pip install autobahntestsuite
//! ```
//! Start echo_server:
//! ```shell
//! cargo run --bin echo_server
//! ```
//! In another terminal run server tests and view results:
//! ```shell
//! cd autobahn
//! wstest -m fuzzingclient
//! open reports/server/index.html
//! ```
//!
//! For testing client implementation first start autobahn server suite:
//! ```shell
//! wstest -m fuzzingserver
//! ```
//! Then in another terminal run client tests and view results:
//! ```shell
//! cargo run --bin autobahn_client
//! open autobahn/reports/client/index.html
//! ```
//! For development purpose there is automation for running autobahn test suite
//! and showing results:
//! ```shell
//! cargo run --bin autobahn_server_test
//! ```
//! you can use run that in development on every file change with cargo-watch:
//! ```shell
//! cargo watch -x 'run --bin autobahn_server_test'
//! ```
//!
//! # Chat server example
//! Simple example of server accepting text messages and distributing them to
//! the all connected clients.  
//! First start chat server:
//! ```shell
//! cargo run --bin chat_server
//! ```
//! Then in browser development console connect to the server and send chat
//! messages:
//! ```javascript
//! var socket = new WebSocket('ws://127.0.0.1:9001');
//! var msgNo = 0;
//! var interval;
//! socket.addEventListener('open', function (event) {
//!     console.log('open');
//!     socket.send("new client");
//!     interval = setInterval(function() {
//!         msgNo++;
//!         socket.send("message: " + msgNo);
//!     }, 1000);
//! });
//! socket.addEventListener('message', function (event) {
//!     console.log('chat', event.data);
//! });
//! socket.addEventListener('close', function (event) {
//!     console.log('closed');
//!     clearInterval(interval);
//! });
//! ```
//! Start multiple browser tabs with the same code running.  
//! You can disconnect from the server with: `socket.close();`.
//!
//! # References
//! [WebSocket Protocol] IETF RFC 6455  
//! [MDN writing WebSocket servers]  
//!
//! [WebSocket Protocol]: https://tools.ietf.org/html/rfc6455
//! [MDN writing WebSocket servers]: https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API/Writing_WebSocket_servers
//!
//! [`Socket`]: struct.Socket.html
//! [`accept`]: struct.Server.html#method.accept
//! [examples/client.rs]: https://github.com/ianic/yarws/blob/master/examples/client.rs
//! [examples/echo_server.rs]: https://github.com/ianic/yarws/blob/master/examples/echo_server.rs
//! [websocat]: https://github.com/vi/websocat
//! [wstest]: https://github.com/crossbario/autobahn-testsuite
//! [autobahn]: https://github.com/crossbario/autobahn-testsuite
//! [cargo-watch]: https://github.com/passcod/cargo-watch
//! [Tokio]: https://tokio.rs
use native_tls;
use slog::Logger;
use std::str;
use tokio;
use tokio::io;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::spawn;
use tokio::stream::StreamExt;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_tls::TlsStream;

#[macro_use]
extern crate slog;
#[macro_use]
extern crate failure;

#[cfg(test)]
#[macro_use]
extern crate hex_literal;

mod http;
pub mod log;
mod stream;
mod ws;
use stream::Stream;

/// Binds tcp listener to the provided addr (typically ip:port).  
///
/// Errors if binding can't be started. In most cases because port is
/// already used, but other errors could occur also; too many open files,
/// incorrect addr.
pub async fn bind<L: Into<Option<slog::Logger>>>(addr: &str, log: L) -> Result<Server, Error> {
    let log = log.into().unwrap_or(log::null());
    let listener = TcpListener::bind(addr).await?;
    Ok(Server {
        rx: Server::listen(listener, log).await,
    })
}

/// Connects to the WebSocket server and on success returns `Socket`.
pub async fn connect<L: Into<Option<slog::Logger>>>(url: &str, log: L) -> Result<Socket, Error> {
    let log = log.into().unwrap_or(log::null());
    let url = parse_url(url)?;
    let tcp_stream = TcpStream::connect(&url.addr).await?; // establish tcp connection
    if url.wss {
        let tls_stream = connect_tls(tcp_stream, &url).await?; // tcp -> tls
        return Ok(connect_stream(tls_stream, &url, log).await?);
    }
    Ok(connect_stream(tcp_stream, &url, log).await?)
}

async fn connect_tls(tcp_stream: TcpStream, url: &Url) -> Result<TlsStream<TcpStream>, Error> {
    let connector = native_tls::TlsConnector::new()?;
    let connector = tokio_tls::TlsConnector::from(connector);
    let stream = connector.connect(&url.domain, tcp_stream).await?;
    Ok(stream)
}

async fn connect_stream<T>(raw_stream: T, url: &Url, log: Logger) -> Result<Socket, Error>
where
    T: AsyncWrite + AsyncRead + std::marker::Send + 'static,
{
    let stream = Stream::new(raw_stream);
    let (stream, deflate_supported) = http::connect(stream, &url).await?; // upgrade tcp to ws
    let (rx, tx) = ws::start(stream, true, deflate_supported, log.clone()).await; // start ws
    return Ok(Socket { rx, tx, no: 1 });
}

/// Represent a WebSocket connection. Used for sending and receiving messages.  
#[derive(Debug)]
pub struct Socket {
    pub no: usize,
    tx: Sender<ws::Msg>,
    rx: Receiver<ws::Msg>,
}

impl Socket {
    /// Receives Msg from the other side of the Socket connection.
    /// None is returned if the socket is closed.
    ///
    /// # Examples
    ///
    /// Usually used in while loop:
    /// ```no-run
    /// while let Some(msg) = socket.recv().await {
    ///     // process msg
    /// }
    /// ```
    pub async fn recv(&mut self) -> Option<Msg> {
        Socket::recv_one(&mut self.rx, &mut self.tx, false).await
    }

    async fn recv_one(rx: &mut Receiver<ws::Msg>, tx: &mut Sender<ws::Msg>, text_only: bool) -> Option<Msg> {
        loop {
            match rx.recv().await {
                None => return None, // channel exhausted
                Some(ws_msg) => match ws_msg {
                    ws::Msg::Text(text) => return Some(Msg::Text(text)),
                    ws::Msg::Binary(payload) => {
                        if text_only {
                            // send close and return
                            if let Err(_) = tx.send(ws::Msg::Close(0)).await {}
                            return None;
                        }
                        return Some(Msg::Binary(payload));
                    }
                    ws::Msg::Close(_) => {
                        if let Err(_) = tx.send(ws_msg).await {}
                        return None;
                    }
                    ws::Msg::Ping(payload) => {
                        if let Err(_) = tx.send(ws::Msg::Pong(payload)).await {
                            return None;
                        }
                    }
                    ws::Msg::Pong(_) => (),
                },
            }
        }
    }

    /// Sends Msg to the other side of the Socket connection.
    /// Errors if the socket is already closed.
    ///
    /// # Examples
    /// Echo example.
    /// Receive Msgs and replay with the same Msg.
    /// ```no-run
    /// async fn echo(mut socket: Socket) -> Result<(), Error> {
    ///     while let Some(msg) = socket.recv().await {
    ///         socket.send(msg).await?;
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub async fn send(&mut self, msg: Msg) -> Result<(), Error> {
        self.tx.send(msg.into_ws_msg()).await?;
        Ok(())
    }

    /// Transforms Socket into pair of mpsc channels for sending/receiving Msgs.
    ///
    /// In some cases it is more convenient to have channels instead of calling
    /// methods. Ownership of each side of the channel can be moved to the
    /// different function.
    pub async fn into_channel(self) -> (Sender<Msg>, Receiver<Msg>) {
        let (tx, mut i_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(1);
        let (mut i_tx, rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(1);

        let mut ws_rx = self.rx;
        let mut ws_tx = self.tx.clone();
        spawn(async move {
            while let Some(msg) = Socket::recv_one(&mut ws_rx, &mut ws_tx, false).await {
                if let Err(_) = i_tx.send(msg).await {
                    break;
                }
            }
        });

        let mut ws_tx = self.tx;
        spawn(async move {
            while let Some(msg) = i_rx.recv().await {
                if let Err(_) = ws_tx.send(msg.into_ws_msg()).await {
                    break;
                }
            }
        });

        (tx, rx)
    }

    /// Transforms Socket into TextSocket which is more convenient for handling
    /// text only messages.
    pub fn into_text(self) -> TextSocket {
        TextSocket {
            no: self.no,
            tx: self.tx,
            rx: self.rx,
        }
    }
}

/// Represent a WebSocket connection. Used for sending and receiving text only
/// messages.
///
/// Each incoming message is transformed into String. Eventual other types of
/// the messages (binary) are ignored.
pub struct TextSocket {
    pub no: usize,
    tx: Sender<ws::Msg>,
    rx: Receiver<ws::Msg>,
}

impl TextSocket {
    pub async fn send(&mut self, text: &str) -> Result<(), Error> {
        self.tx.send(ws::Msg::Text(text.to_owned())).await?;
        Ok(())
    }

    /// Receives String from the other side of the Socket connection.
    /// None is returned if the socket is closed.
    pub async fn recv(&mut self) -> Option<String> {
        TextSocket::recv_one(&mut self.rx, &mut self.tx).await
    }

    /// Sends String to the other side of the Socket connection.
    /// Errors if the socket is already closed.
    pub async fn try_recv(&mut self) -> Result<String, Error> {
        match self.recv().await {
            None => Err(Error::SocketClosed),
            Some(v) => Ok(v),
        }
    }

    async fn recv_one(mut rx: &mut Receiver<ws::Msg>, mut tx: &mut Sender<ws::Msg>) -> Option<String> {
        match Socket::recv_one(&mut rx, &mut tx, true).await {
            Some(Msg::Text(text)) => return Some(text),
            _ => None,
        }
    }

    /// Transforms Socket into pair of mpsc channels for sending/receiving
    /// Strings.
    pub async fn into_channel(self) -> (Sender<String>, Receiver<String>) {
        let (tx, mut i_rx): (Sender<String>, Receiver<String>) = mpsc::channel(1);
        let (mut i_tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel(1);

        let mut ws_rx = self.rx;
        let mut ws_tx = self.tx.clone();
        spawn(async move {
            while let Some(text) = TextSocket::recv_one(&mut ws_rx, &mut ws_tx).await {
                if let Err(_) = i_tx.send(text).await {
                    break;
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

/// Message exchanged between library and application.
///
/// Can be text or binary. Text messages are valid UTF-8 strings. Binary of
/// course can be anything. Web servers will typically send text messages.
pub enum Msg {
    Text(String),
    Binary(Vec<u8>),
}

impl Msg {
    /// Converts to Msg used in `ws` module.
    fn into_ws_msg(self) -> ws::Msg {
        match self {
            Msg::Text(text) => ws::Msg::Text(text),
            Msg::Binary(vec) => ws::Msg::Binary(vec),
        }
    }
}

/// For creating WebSocket servers.
pub struct Server {
    rx: Receiver<Socket>,
}

impl Server {
    /// Returns `Socket` for successfully established WebSocket connection.
    /// Loop over this method to handle all incoming connections.  
    pub async fn accept(&mut self) -> Option<Socket> {
        self.rx.recv().await
    }

    // Listens for incoming tcp connections. Upgrades them to WebSocket and
    // feeds socket_tx channel with Socket for each established connection.
    async fn listen(mut listener: TcpListener, log: Logger) -> Receiver<Socket> {
        let (socket_tx, socket_rx): (Sender<Socket>, Receiver<Socket>) = mpsc::channel(1);

        spawn(async move {
            let mut conn_no = 0;
            let mut incoming = listener.incoming();
            while let Some(conn) = incoming.next().await {
                match conn {
                    Ok(stream) => {
                        conn_no += 1;
                        let log = log.new(o!("conn" => conn_no));
                        spawn_accept(stream, socket_tx.clone(), conn_no, log).await;
                    }
                    Err(e) => error!(log, "accept error: {}", e),
                }
            }
        });

        socket_rx
    }
}

async fn spawn_accept(stream: TcpStream, socket_tx: Sender<Socket>, no: usize, log: Logger) {
    spawn(async move {
        if let Err(e) = accept(stream, socket_tx, no, log.clone()).await {
            error!(log, "{}", e);
        }
    });
}

// Upgrades tcp connection to the WebSocket, starts ws handler and returns new
// Socket through socket_tx channel.
async fn accept(tcp_stream: TcpStream, mut socket_tx: Sender<Socket>, no: usize, log: Logger) -> Result<(), Error> {
    let stream = Stream::new(tcp_stream);
    let (stream, deflate_supported) = http::accept(stream).await?;
    let (rx, tx) = ws::start(stream, false, deflate_supported, log).await;
    let socket = Socket { no, tx, rx };
    socket_tx.send(socket).await?;
    Ok(())
}

#[derive(Fail, Debug)]
/// Definition of all errors returned from the library.
pub enum Error {
    #[fail(display = "invalid upgrade request")]
    InvalidUpgradeRequest,
    #[fail(display = "IO error: {}", error)]
    IoError { error: io::Error },

    #[fail(display = "fail to send Msg: {}", error)]
    MsgSendError { error: mpsc::error::SendError<ws::Msg> },
    #[fail(display = "fail to send bytes: {}", error)]
    RawSendError { error: mpsc::error::SendError<Vec<u8>> },
    #[fail(display = "fail to send socket: {}", error)]
    SocketSendError { error: mpsc::error::SendError<Socket> },

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
    #[fail(display = "tls error: {}", error)]
    TlsError { error: native_tls::Error },
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::IoError { error: e }
    }
}
impl From<mpsc::error::SendError<ws::Msg>> for Error {
    fn from(e: mpsc::error::SendError<ws::Msg>) -> Self {
        Error::MsgSendError { error: e }
    }
}
impl From<mpsc::error::SendError<Vec<u8>>> for Error {
    fn from(e: mpsc::error::SendError<Vec<u8>>) -> Self {
        Error::RawSendError { error: e }
    }
}
impl From<mpsc::error::SendError<Socket>> for Error {
    fn from(e: mpsc::error::SendError<Socket>) -> Self {
        Error::SocketSendError { error: e }
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(e: std::str::Utf8Error) -> Self {
        Error::TextPayloadNotValidUTF8(e)
    }
}

impl From<native_tls::Error> for Error {
    fn from(e: native_tls::Error) -> Self {
        Error::TlsError { error: e }
    }
}

pub struct Url {
    addr: String,
    path: String,
    domain: String,
    wss: bool,
}

fn parse_url(u: &str) -> Result<Url, Error> {
    let url = match match url::Url::parse(u) {
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let url = "ws://".to_owned() + u;
            url::Url::parse(&url)
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
    let wss = url.scheme() == "wss";
    let u = Url {
        wss: wss,
        addr: addr,
        path: path,
        domain: host.to_owned(),
    };
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_url() {
        let url = parse_url("ws://localhost:9001/path?pero=zdero").unwrap();
        assert_eq!("localhost:9001", url.addr);
        assert_eq!("/path?pero=zdero", url.path);
        assert!(!url.wss);

        let url = parse_url("ws://localhost:9001/path").unwrap();
        assert_eq!("localhost:9001", url.addr);
        assert_eq!("/path", url.path);
        assert!(!url.wss);

        let url = parse_url("ws://localhost/path").unwrap();
        assert_eq!("localhost:80", url.addr);
        assert_eq!("/path", url.path);
        assert!(!url.wss);

        let url = parse_url("localhost/path").unwrap();
        assert_eq!("localhost:80", url.addr);
        assert_eq!("/path", url.path);
        assert!(!url.wss);

        let url = parse_url("pero://localhost/path").unwrap();
        assert_eq!("localhost:0", url.addr);
        assert_eq!("/path", url.path);
        assert!(!url.wss);

        let url = parse_url("wss://localhost:9001/path").unwrap();
        assert_eq!("localhost:9001", url.addr);
        assert_eq!("/path", url.path);
        assert!(url.wss);

        let url = parse_url("wss://echo.websocket.org/path").unwrap();
        assert_eq!("echo.websocket.org:443", url.addr);
        assert_eq!("echo.websocket.org", url.domain);
        assert_eq!("/path", url.path);
        assert!(url.wss);
    }
}
