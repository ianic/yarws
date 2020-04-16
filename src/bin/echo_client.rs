use slog::Logger;
use structopt::StructOpt;
use yarws::{connect, log, Error, Socket};

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
        format!("{}:{}/getCaseCount", self.ip, self.port)
    }
}

#[tokio::main]
async fn main() {
    let args = Args::from_args();
    let log = log::config();

    let socket = match connect(&args.addr(), log.clone()).await {
        Ok(s) => s,
        Err(e) => {
            error!(log, "{}", e);
            return;
        }
    };

    if let Err(e) = handler(socket, log.clone()).await {
        error!(log, "{}", e);
    }
}

async fn handler(mut s: Socket, _log: Logger) -> Result<(), Error> {
    while let Some(msg) = s.recv().await {
        s.send(msg).await?;
    }
    Ok(())
}
