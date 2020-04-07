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
use slog::Logger;
use std::error::Error;
use tokio::sync::mpsc::{Receiver, Sender};
#[macro_use]
extern crate failure;

mod http;
mod ws;

#[tokio::main]
async fn main() {
    // configure logging
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    //let log Logger::root(drain, o!());
    let log = Logger::root(
        drain,
        o!("file" =>
         slog::FnValue(move |info| {
             format!("{}:{} {}",
                     info.file(),
                     info.line(),
                     info.module(),
                     )
         })
        ),
    );

    let addr = "127.0.0.1:9001";

    let clog = log.new(o!("addr" => addr));
    let mut listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(clog, "{}", e);
            return;
        }
    };
    info!(clog, "started");

    let mut conn_no = 0;

    let server = {
        async move {
            let mut incoming = listener.incoming();
            while let Some(conn) = incoming.next().await {
                match conn {
                    Err(e) => error!(log, "accept error: {}", e),
                    Ok(sock) => {
                        conn_no += 1;
                        let l = log.new(o!("conn" => conn_no));
                        let sl = log.new(o!("conn" => conn_no));
                        spawn(async move {
                            match http::upgrade(sock).await {
                                Err(e) => error!(l, "upgrade error: {}", e),
                                Ok(u) => {
                                    let (rx, tx) = ws::handle(u, l).await;
                                    session(rx, tx, sl).await.unwrap();
                                }
                            }
                        });
                    }
                }
            }
        }
    };
    server.await;
}

async fn session(mut rx: Receiver<ws::Msg>, mut tx: Sender<ws::Msg>, log: slog::Logger) -> Result<(), Box<dyn Error>> {
    while let Some(m) = rx.recv().await {
        // match m {
        //     ws::Msg::Text(t) => {
        //         let t = t.chars().rev().collect::<String>();
        //         tx.send(ws::Msg::Text(t)).await?;
        //     }
        //     _ => tx.send(m).await?,
        // }
        tx.send(m).await?;
    }
    debug!(log, "session closed");
    Ok(())
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
