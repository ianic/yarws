//use std::str;
use slog::Drain;
use slog::Logger;
use tokio::net::TcpStream;

#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let log = log_config();
    let stream = TcpStream::connect("127.0.0.1:9001").await?;

    match yarws::http::connect(stream).await {
        Ok(u) => {
            let (mut rx, mut tx) = yarws::ws::handle(u, log.clone()).await;
            tx.send(yarws::Msg::Text("pero".to_owned())).await?;
            match rx.recv().await.unwrap() {
                yarws::Msg::Text(t) => println!("response: {}", t),
                _ => println!("opaaaa"),
            }
        }
        Err(e) => error!(log, "upgrade error: {}", e),
    }

    //println!("{:?}", connect);
    //loop {

    // tokio::spawn(async move {
    //     //loop {
    //     println!("{}", http());
    //     if let Err(e) = socket.write_all(http().as_bytes()).await {
    //         eprintln!("failed to write to socket; err = {:?}", e);
    //         return;
    //     }

    //     loop {
    //         let mut buf = [0; 1024];
    //         match socket.read(&mut buf).await {
    //             // socket closed
    //             Ok(n) if n == 0 => return,
    //             Ok(n) => {
    //                 println!("received {} bytes", n);
    //                 println!("{}", str::from_utf8(&buf).unwrap());
    //             }
    //             Err(e) => {
    //                 eprintln!("failed to read from socket; err = {:?}", e);
    //                 return;
    //             }
    //         };
    //     }
    //     //}
    // })
    // .await;
    //}
    Ok(())
}

fn log_config() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    //let log Logger::root(drain, o!());
    Logger::root(
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
    )
}

// fn http() -> String {
//     let header = "GET / HTTP/1.1\r\n\
// Connection: Upgrade\r\n\
// Pragma: no-cache\r\n\
// Host: localhost:9001\r\n\
// Cache-Control: no-cache\r\n\
// Upgrade: websocket\r\n\
// Origin: http://localhost:9001\r\n\
// Sec-WebSocket-Version: 13\r\n\
// Sec-WebSocket-Key: HNDy4+PhhRtPmNt1Xet/Ew==\r\n\
// Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits\r\n\r\n";
//     header.to_owned()
// }
