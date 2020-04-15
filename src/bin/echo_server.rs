use structopt::StructOpt;
use tokio;
use tokio::sync::mpsc::{Receiver, Sender};
use yarws::{log, Error, Msg, Server};

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
    let status = {
        let args = Args::from_args();
        let log = log::config();
        if let Err(e) = run(&args, log.clone()).await {
            error!(log, "{}", e; "addr" => args.addr());
            1
        } else {
            0
        }
    };
    std::process::exit(status);
}

async fn run(args: &Args, log: slog::Logger) -> Result<(), Error> {
    let mut srv_rx = Server::bind(&args.addr(), log.clone()).await?;
    while let Some(socket) = srv_rx.recv().await {
        if args.reverse {
            reverse_echo(socket.rx, socket.tx).await?;
        } else {
            echo(socket.rx, socket.tx).await?;
        };
        trace!(log, "socket closed"; "conn" => socket.no);
    }
    Ok(())
}

async fn echo(mut rx: Receiver<Msg>, mut tx: Sender<Msg>) -> Result<(), Error> {
    while let Some(m) = rx.recv().await {
        tx.send(m).await?;
    }
    Ok(())
}

async fn reverse_echo(mut rx: Receiver<Msg>, mut tx: Sender<Msg>) -> Result<(), Error> {
    while let Some(m) = rx.recv().await {
        let m = match m {
            Msg::Text(t) => {
                let t = t.chars().rev().collect::<String>();
                Msg::Text(t)
            }
            _ => m,
        };
        tx.send(m).await?;
    }
    Ok(())
}
