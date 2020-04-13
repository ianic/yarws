use structopt::StructOpt;
use tokio;
use tokio::sync::mpsc::{Receiver, Sender};

#[macro_use]
extern crate slog;

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(short = "p", long = "port", default_value = "9001")]
    port: usize,

    #[structopt(short = "i", long = "bind-ip", default_value = "127.0.0.1")]
    ip: String,

    #[structopt(short = "r", long = "reverse", help = "Reverse every text message")]
    reverse: bool,
}

impl Args {
    fn addr(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }
}

#[tokio::main]
async fn main() {
    let args = Args::from_args();
    let log = yarws::log::config();

    match yarws::Server::bind(&args.addr(), log.clone()).await {
        Ok(srv) => {
            let mut socket = srv.listen().await;
            while let Some(socket) = socket.recv().await {
                let res = if args.reverse {
                    reverse_echo(socket.rx, socket.tx).await
                } else {
                    echo(socket.rx, socket.tx).await
                };
                if let Err(e) = res {
                    error!(log, "socket error: {}", e);
                } else {
                    trace!(log, "socket closed"; "conn" => socket.no);
                }
            }
        }
        Err(e) => {
            error!(log, "failed to start server error: {}", e; "addr" => args.addr());
            std::process::exit(-1);
        }
    }
}

async fn echo(mut rx: Receiver<yarws::Msg>, mut tx: Sender<yarws::Msg>) -> Result<(), yarws::Error> {
    while let Some(m) = rx.recv().await {
        tx.send(m).await?;
    }
    Ok(())
}

async fn reverse_echo(mut rx: Receiver<yarws::Msg>, mut tx: Sender<yarws::Msg>) -> Result<(), yarws::Error> {
    while let Some(m) = rx.recv().await {
        let m = match m {
            yarws::Msg::Text(t) => {
                let t = t.chars().rev().collect::<String>();
                yarws::Msg::Text(t)
            }
            _ => m,
        };
        tx.send(m).await?;
    }
    Ok(())
}
