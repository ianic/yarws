// server.rs
use tokio;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::stream::StreamExt;
#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;
use slog::Drain;

mod http;
mod ws;

#[tokio::main]
async fn main() {
    // configure logging
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let log = slog::Logger::root(drain, o!());

    info!(log, "starting");
    let mut listener = TcpListener::bind("127.0.0.1:9001").await.unwrap();

    let server = {
        async move {
            let mut incoming = listener.incoming();
            while let Some(conn) = incoming.next().await {
                match conn {
                    Err(e) => error!(log, "accept failed"; "error" => format!("{:?}", e)),
                    Ok(sock) => {
                        let l = log.clone();
                        spawn(async move {
                            match http::upgrade(sock).await {
                                Err(e) => error!(l, "upgrade"; "error" => format!("{:?}", e)),
                                Ok(ws_sock) => ws::handle(ws_sock, l).await,
                            }
                        });
                    }
                }
            }
        }
    };
    server.await;
}

#[cfg(test)]
#[macro_use]
extern crate hex_literal;

/*
example of ws upgrade header:

16 GET / HTTP/1.1
22 Host: localhost:8000
21 Connection: Upgrade
18 Pragma: no-cache
25 Cache-Control: no-cache
135 User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/80.0.3987.149 Safari/537.36
20 Upgrade: websocket
31 Origin: http://localhost:8000
27 Sec-WebSocket-Version: 13
36 Accept-Encoding: gzip, deflate, br
114 Accept-Language: en-US,en;q=0.9,de;q=0.8,et;q=0.7,hr;q=0.6,it;q=0.5,sk;q=0.4,sl;q=0.3,sr;q=0.2,bs;q=0.1,mt;q=0.1
45 Sec-WebSocket-Key: HNDy4+PhhRtPmNt1Xet/Ew==
70 Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits
*/
/*

var socket = new WebSocket('ws://localhost:8000');
socket.addEventListener('open', function (event) {
    console.log('open');
    //socket.send('first');
    //socket.send('second');
    //socket.send('third');
    socket.close();
});
socket.addEventListener('message', function (event) {
    console.log('Message from server ', event.data);
});
socket.addEventListener('close', function (event) {
    console.log('ws closed');
});
socket.addEventListener('error', function (event) {
  console.log('ws error: ', event);
});


|Refactoring example

// my first implementation
    let l = frame.header_len() as usize + 2;
    if l > 0 {
        input.read_exact(&mut buf[2..l]).await?;
        frame.set_header(&buf[2..l]);
    }

// trying to put l into smaller scope
    match frame.header_len() as usize + 2 {
        0 => (),
        l => {
            input.read_exact(&mut buf[2..l]).await?;
            frame.set_header(&buf[2..l]);
        }
    }

// after refactoring header_len to return Option
    if let Some(l) = frame.header_len2() {
        input.read_exact(&mut buf[2..l]).await?;
        frame.set_header(&buf[2..l]);
    }


,
      "1.*",
      "2.*",
      "3.*",
      "4.*",
      "5.*",
      "6.*",
      "7.*"


*/
