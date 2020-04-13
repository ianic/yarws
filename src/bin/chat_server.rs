use structopt::StructOpt;
use tokio;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use yarws::{Error, Msg, Server};

#[macro_use]
extern crate slog;

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(short = "p", long = "port", default_value = "9001")]
    port: usize,

    #[structopt(short = "i", long = "bind-ip", default_value = "127.0.0.1")]
    ip: String,
}

impl Args {
    fn addr(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }
}

#[tokio::main]
async fn main() {
    let status = {
        let args = Args::from_args();
        let log = yarws::log::config();

        if let Err(e) = run(&args.addr(), log.clone()).await {
            error!(log, "{}", e; "addr" => args.addr());
            1
        } else {
            0
        }
    };
    // exit must be called after logger goes out of scope
    // https://docs.rs/slog-async/2.5.0/slog_async/#structs
    std::process::exit(status);
}

async fn run(addr: &str, log: slog::Logger) -> Result<(), Error> {
    let srv = Server::bind(addr, log.clone()).await?;
    let mut socket = srv.listen().await;

    let (mut sub_tx, room_tx) = chat().await;
    while let Some(socket) = socket.recv().await {
        if let Err(_e) = sub_tx.send(socket.tx).await {
            break;
        }
        client(room_tx.clone(), socket.rx).await;
    }

    Ok(())
}

async fn client(mut room_tx: Sender<Msg>, mut socket_rx: Receiver<Msg>) {
    spawn(async move {
        while let Some(m) = socket_rx.recv().await {
            if let Err(_e) = room_tx.send(m).await {
                return;
            }
        }
    });
}

// async fn echo(mut rx: Receiver<Msg>, mut tx: Sender<Msg>) -> Result<(), Error> {
//     while let Some(m) = rx.recv().await {
//         tx.send(m).await?;
//     }
//     Ok(())
// }

async fn chat() -> (Sender<Sender<Msg>>, Sender<Msg>) {
    // room
    let (room_tx, mut room_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(16);
    let (mut broker_tx, mut broker_rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel(16);
    let (sub_tx, mut sub_rx): (Sender<Sender<Msg>>, Receiver<Sender<Msg>>) = mpsc::channel(16);

    spawn(async move {
        while let Some(m) = room_rx.recv().await {
            if let Err(_e) = broker_tx.send(m).await {
                return;
            }
        }
    });

    // broker
    spawn(async move {
        let mut subscribers: Vec<Sender<Msg>> = Vec::new();
        loop {
            tokio::select! {
             Some(sub) = sub_rx.recv() => {  // new subscriber
               subscribers.push(sub);
             },
            Some(msg) = broker_rx.recv() => {  // new message from the chat room, fan it out to all subscribers
               let mut gone: Vec<usize> = Vec::new();
               for (idx, sub) in subscribers.iter_mut().enumerate() {
                   if let Err(_e) = sub.send(msg.clone()).await { // todo: try write maybe?
                       gone.push(idx);
                   }
               }
              gone.reverse();
               for idx in gone {
                   subscribers.remove(idx);
               }
              },
              else => {
                  break;
              }
            }
        }
    });

    (sub_tx, room_tx)
}

/*
// Dummy Javascript client:

var socket = new WebSocket('ws://127.0.0.1:9001');
var msgNo = 0;
var clientId = new Date().getTime();
socket.addEventListener('open', function (event) {
    console.log('open');
    socket.send("new client: " + clientId);
    setInterval(function() {
        msgNo++;
        socket.send("client: " + clientId + " message: " + msgNo);
    }, 1000);
});
socket.addEventListener('message', function (event) {
    console.log('chat', event.data);
});
socket.addEventListener('close', function (event) {
    console.log('ws closed');
});
socket.addEventListener('error', function (event) {
  console.log('ws error: ', event);
});
*/
