use structopt::StructOpt;
use tokio;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use yarws::{Error, Server, TextSocket};

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
    let mut srv = Server::bind(addr, log.clone()).await?;
    let chat_tx = chat().await;

    while let Some(socket) = srv.accept().await {
        client(socket.into_text(), chat_tx.clone()).await;
    }

    Ok(())
}

async fn client(socket: TextSocket, mut chat_tx: Sender<Subscription>) {
    let client_id = socket.no;
    let (tx, mut rx) = socket.into_channel().await;

    let sub = Subscription::Subscribe(Subscriber {
        client_id: client_id,
        tx: tx,
    });
    if let Err(_e) = chat_tx.send(sub).await {
        return;
    }

    spawn(async move {
        while let Some(text) = rx.recv().await {
            let msg = format!("{} {}", client_id, text);
            if let Err(_e) = chat_tx.send(Subscription::Post(msg)).await {
                break;
            }
        }
        chat_tx
            .send(Subscription::Unsubscribe(client_id))
            .await
            .unwrap_or_default();
    });
}

struct Subscriber {
    client_id: usize,
    tx: Sender<String>,
}

enum Subscription {
    Subscribe(Subscriber),
    Post(String),
    Unsubscribe(usize),
}

async fn chat() -> Sender<Subscription> {
    // room
    let (mut room_tx, mut room_rx): (Sender<String>, Receiver<String>) = mpsc::channel(1);
    let (mut broker_tx, mut broker_rx): (Sender<String>, Receiver<String>) = mpsc::channel(1);
    let (sub_tx, mut sub_rx): (Sender<Subscription>, Receiver<Subscription>) = mpsc::channel(1);

    spawn(async move {
        while let Some(text) = room_rx.recv().await {
            if let Err(_e) = broker_tx.send(text).await {
                return;
            }
        }
    });

    // broker
    spawn(async move {
        let mut subscribers: Vec<Subscriber> = Vec::new();
        loop {
            tokio::select! {
             Some(sub) = sub_rx.recv() => {  // new subscriber
                match sub {
                    Subscription::Subscribe(s) => subscribers.push(s),
                    Subscription::Unsubscribe(client_id) => {
                        for (idx, sub) in subscribers.iter().enumerate() {
                            if sub.client_id == client_id {
                                subscribers.remove(idx);
                                break;
                            }
                        }
                    }
                    Subscription::Post(text) => {
                        room_tx.send(text).await.unwrap_or_default();
                    }
                }
             },
            Some(msg) = broker_rx.recv() => {  // new message from the chat room, fan it out to all subscribers
               let mut gone: Vec<usize> = Vec::new();
               for (idx, sub) in subscribers.iter_mut().enumerate() {
                   if let Err(_e) = sub.tx.send(msg.clone()).await { // todo: try write maybe?
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

    sub_tx
}

/*
// Dummy Javascript client:

var socket = new WebSocket('ws://127.0.0.1:9001');
var msgNo = 0;
var interval;
socket.addEventListener('open', function (event) {
    console.log('open');
    socket.send("new client");
    interval = setInterval(function() {
        msgNo++;
        socket.send("message: " + msgNo);
    }, 1000);
});
socket.addEventListener('message', function (event) {
    console.log('chat', event.data);
});
socket.addEventListener('close', function (event) {
    console.log('closed');
    clearInterval(interval);
});



*/
