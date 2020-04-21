use structopt::StructOpt;
use tokio;
use tokio::spawn;
use yarws::{bind, log, Error, Socket};

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
async fn main() -> Result<(), Error> {
    let args = Args::from_args();
    let log = log::config();

    // bind to tcp port
    let mut srv = bind(&args.addr(), log.clone()).await?;
    // wait for incoming connection
    while let Some(socket) = srv.accept().await {
        let log = log.new(o!("conn" => socket.no));
        // spawn task for handling socket
        spawn(async move {
            if let Err(e) = echo(socket).await {
                error!(log, "{}", e);
            }
        });
    }

    Ok(())
}

async fn echo(mut socket: Socket) -> Result<(), Error> {
    // read messages until connection is closed
    while let Some(msg) = socket.recv().await {
        // reply with the same message
        socket.send(msg).await?;
    }
    Ok(())
}
