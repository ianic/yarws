use structopt::StructOpt;
use tokio;
use tokio::spawn;
use yarws::{log, Error, Msg, Server, Socket};

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
    let mut srv = Server::bind(&args.addr(), log.clone()).await?;
    let reverse = args.reverse;
    while let Some(socket) = srv.accept().await {
        let log = log.new(o!("conn" => socket.no));
        spawn(async move {
            if reverse {
                if let Err(e) = reverse_echo(socket).await {
                    error!(log, "{}", e);
                }
            } else {
                if let Err(e) = echo(socket).await {
                    error!(log, "{}", e);
                }
            };
            trace!(log, "socket closed");
        });
    }
    Ok(())
}

async fn echo(mut socket: Socket) -> Result<(), Error> {
    while let Some(msg) = socket.recv().await {
        socket.send(msg).await?;
    }
    Ok(())
}

async fn reverse_echo(mut socket: Socket) -> Result<(), Error> {
    while let Some(msg) = socket.recv().await {
        let msg = match msg {
            Msg::Text(t) => {
                let t = t.chars().rev().collect::<String>();
                Msg::Text(t)
            }
            _ => msg,
        };
        socket.send(msg).await?;
    }
    Ok(())
}
