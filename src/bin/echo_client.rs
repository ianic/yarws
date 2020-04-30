use structopt::StructOpt;
use yarws::{connect, log, Error};

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
    let status = {
        let args = Args::from_args();
        let log = log::config();
        if let Err(e) = run(&args.addr()).await {
            error!(log, "{}", e; "addr" => args.addr());
            1
        } else {
            0
        }
    };
    std::process::exit(status);
}

async fn run(addr: &str) -> Result<(), Error> {
    let mut socket = connect(addr).await?;
    while let Some(msg) = socket.recv().await {
        socket.send(msg).await?;
    }
    Ok(())
}
