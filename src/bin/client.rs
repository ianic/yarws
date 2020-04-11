use slog::Logger;
use structopt::StructOpt;

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
    let args = Args::from_args();
    let log = yarws::log::config();

    let socket = match yarws::connect(&args.addr(), log.clone()).await {
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

async fn handler(mut socket: yarws::Socket, log: Logger) -> Result<(), yarws::Error> {
    socket.send("pero zdero").await?;
    if let Some(text) = socket.receive().await? {
        debug!(log, "{}", text);
    }
    Ok(())
}
